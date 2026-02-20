package config

import (
	"errors"
	"os"
	"path/filepath"
	"testing"
)

func TestSaveAndLoad(t *testing.T) {
	t.Parallel()

	tempDir := t.TempDir()
	path := filepath.Join(tempDir, "state", "config.json")

	cfg := File{
		MasterProvider: "codex",
		Providers: []Provider{
			{Name: "codex", Installed: true, AuthStatus: "logged_in"},
			{Name: "claude", Installed: true, AuthStatus: "unknown"},
		},
	}
	if err := Save(path, cfg); err != nil {
		t.Fatalf("save failed: %v", err)
	}

	loaded, err := Load(path)
	if err != nil {
		t.Fatalf("load failed: %v", err)
	}
	if loaded.MasterProvider != "codex" {
		t.Fatalf("unexpected master provider: %s", loaded.MasterProvider)
	}
	if loaded.Version != CurrentVersion {
		t.Fatalf("unexpected version: %d", loaded.Version)
	}
	if loaded.CreatedAt.IsZero() || loaded.UpdatedAt.IsZero() {
		t.Fatal("timestamps should be populated")
	}
}

func TestLoadMissing(t *testing.T) {
	t.Parallel()

	_, err := Load(filepath.Join(t.TempDir(), "missing.json"))
	if err == nil {
		t.Fatal("expected missing file error")
	}
	if !errors.Is(err, os.ErrNotExist) {
		t.Fatalf("expected os.ErrNotExist, got: %v", err)
	}
}
