package config

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"time"
)

const CurrentVersion = 1

type Provider struct {
	Name                   string   `json:"name"`
	Binary                 string   `json:"binary"`
	Installed              bool     `json:"installed"`
	Version                string   `json:"version,omitempty"`
	AuthStatus             string   `json:"auth_status"`
	SupportsNonInteractive bool     `json:"supports_non_interactive"`
	SupportsJSON           bool     `json:"supports_json"`
	Notes                  []string `json:"notes,omitempty"`
}

type File struct {
	Version        int        `json:"version"`
	MasterProvider string     `json:"master_provider"`
	Providers      []Provider `json:"providers"`
	CreatedAt      time.Time  `json:"created_at"`
	UpdatedAt      time.Time  `json:"updated_at"`
}

func DefaultPath() (string, error) {
	home, err := os.UserHomeDir()
	if err != nil {
		return "", fmt.Errorf("resolve home directory: %w", err)
	}
	return filepath.Join(home, ".orc", "config.json"), nil
}

func ResolvePath(path string) (string, error) {
	trimmed := strings.TrimSpace(path)
	if trimmed == "" {
		return DefaultPath()
	}
	if filepath.IsAbs(trimmed) {
		return filepath.Clean(trimmed), nil
	}
	cwd, err := os.Getwd()
	if err != nil {
		return "", fmt.Errorf("resolve current directory: %w", err)
	}
	return filepath.Join(cwd, trimmed), nil
}

func Save(path string, cfg File) error {
	resolvedPath, err := ResolvePath(path)
	if err != nil {
		return err
	}
	if err := os.MkdirAll(filepath.Dir(resolvedPath), 0o755); err != nil {
		return fmt.Errorf("create config directory: %w", err)
	}

	cfg.Version = CurrentVersion
	if cfg.CreatedAt.IsZero() {
		cfg.CreatedAt = time.Now().UTC()
	}
	cfg.UpdatedAt = time.Now().UTC()

	payload, err := json.MarshalIndent(cfg, "", "  ")
	if err != nil {
		return fmt.Errorf("marshal config: %w", err)
	}
	if err := os.WriteFile(resolvedPath, payload, 0o644); err != nil {
		return fmt.Errorf("write config: %w", err)
	}
	return nil
}

func Load(path string) (File, error) {
	resolvedPath, err := ResolvePath(path)
	if err != nil {
		return File{}, err
	}
	payload, err := os.ReadFile(resolvedPath)
	if err != nil {
		return File{}, fmt.Errorf("read config: %w", err)
	}

	var cfg File
	if err := json.Unmarshal(payload, &cfg); err != nil {
		return File{}, fmt.Errorf("parse config: %w", err)
	}
	return cfg, nil
}
