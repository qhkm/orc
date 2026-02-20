package providers

import (
	"context"
	"strings"
	"time"
)

const codexBinary = "codex"

type CodexProvider struct {
	runner  Runner
	timeout time.Duration
}

func NewCodexProvider() *CodexProvider {
	return &CodexProvider{
		runner:  OSRunner{},
		timeout: 6 * time.Second,
	}
}

func (p *CodexProvider) Name() string {
	return codexBinary
}

func (p *CodexProvider) Doctor(ctx context.Context) DoctorReport {
	report := DoctorReport{
		Name:       p.Name(),
		Binary:     codexBinary,
		AuthStatus: AuthStatusUnknown,
	}

	binaryPath, err := p.runner.LookPath(codexBinary)
	if err != nil {
		report.Error = err.Error()
		report.Notes = append(report.Notes, "binary not found on PATH")
		return report
	}
	report.Installed = true
	report.Binary = binaryPath

	report.Version = p.detectVersion(ctx, binaryPath)
	report.SupportsNonInteractive, report.SupportsJSON = p.detectCapabilities(ctx, binaryPath)
	report.AuthStatus = p.detectAuth(ctx, binaryPath)

	return report
}

func (p *CodexProvider) detectVersion(ctx context.Context, binaryPath string) string {
	cmdCtx, cancel := context.WithTimeout(ctx, p.timeout)
	defer cancel()
	stdout, stderr, err := p.runner.Run(cmdCtx, binaryPath, "--version")
	if err != nil {
		return "unknown"
	}

	for _, line := range cleanedLines(stdout + "\n" + stderr) {
		if strings.Contains(strings.ToLower(line), "codex") {
			return line
		}
	}
	return "unknown"
}

func (p *CodexProvider) detectCapabilities(ctx context.Context, binaryPath string) (bool, bool) {
	cmdCtx, cancel := context.WithTimeout(ctx, p.timeout)
	defer cancel()
	stdout, stderr, err := p.runner.Run(cmdCtx, binaryPath, "exec", "--help")
	if err != nil {
		return false, false
	}
	text := strings.ToLower(strings.Join(cleanedLines(stdout+"\n"+stderr), "\n"))

	nonInteractive := strings.Contains(text, "run codex non-interactively") || strings.Contains(text, "usage: codex exec")
	supportsJSON := strings.Contains(text, "--json") || strings.Contains(text, "jsonl")
	return nonInteractive, supportsJSON
}

func (p *CodexProvider) detectAuth(ctx context.Context, binaryPath string) AuthStatus {
	cmdCtx, cancel := context.WithTimeout(ctx, p.timeout)
	defer cancel()
	stdout, stderr, err := p.runner.Run(cmdCtx, binaryPath, "login", "status")
	if err != nil {
		return AuthStatusError
	}

	text := strings.ToLower(strings.Join(cleanedLines(stdout+"\n"+stderr), "\n"))
	switch {
	case strings.Contains(text, "logged in"):
		return AuthStatusLoggedIn
	case strings.Contains(text, "not logged in"):
		return AuthStatusLoggedOut
	default:
		return AuthStatusUnknown
	}
}

func cleanedLines(text string) []string {
	lines := strings.Split(text, "\n")
	result := make([]string, 0, len(lines))
	for _, raw := range lines {
		line := strings.TrimSpace(raw)
		if line == "" {
			continue
		}
		if strings.HasPrefix(line, "WARNING: proceeding, even though we could not update PATH") {
			continue
		}
		result = append(result, line)
	}
	return result
}
