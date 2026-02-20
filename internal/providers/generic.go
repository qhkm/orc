package providers

import (
	"context"
	"strings"
	"time"
)

type GenericProvider struct {
	name          string
	binary        string
	versionArgs   [][]string
	helpArgs      []string
	runner        Runner
	timeout       time.Duration
	capabilityTag string
}

func NewGenericProvider(name, binary string, versionArgs [][]string, helpArgs []string, capabilityTag string) *GenericProvider {
	return &GenericProvider{
		name:          name,
		binary:        binary,
		versionArgs:   versionArgs,
		helpArgs:      helpArgs,
		runner:        OSRunner{},
		timeout:       4 * time.Second,
		capabilityTag: strings.ToLower(capabilityTag),
	}
}

func (p *GenericProvider) Name() string {
	return p.name
}

func (p *GenericProvider) Doctor(ctx context.Context) DoctorReport {
	report := DoctorReport{
		Name:       p.name,
		Binary:     p.binary,
		AuthStatus: AuthStatusUnknown,
	}

	binaryPath, err := p.runner.LookPath(p.binary)
	if err != nil {
		report.Error = err.Error()
		report.Notes = append(report.Notes, "binary not found on PATH")
		return report
	}

	report.Installed = true
	report.Binary = binaryPath
	report.Version = p.detectVersion(ctx, binaryPath)
	report.SupportsNonInteractive, report.SupportsJSON = p.detectCapabilities(ctx, binaryPath)
	report.Notes = append(report.Notes, "auth check not implemented in MVP")

	return report
}

func (p *GenericProvider) detectVersion(ctx context.Context, binaryPath string) string {
	for _, args := range p.versionArgs {
		cmdCtx, cancel := context.WithTimeout(ctx, p.timeout)
		stdout, stderr, err := p.runner.Run(cmdCtx, binaryPath, args...)
		cancel()
		if err != nil {
			continue
		}
		for _, line := range cleanedLines(stdout + "\n" + stderr) {
			if line != "" {
				return line
			}
		}
	}
	return "unknown"
}

func (p *GenericProvider) detectCapabilities(ctx context.Context, binaryPath string) (bool, bool) {
	if len(p.helpArgs) == 0 {
		return false, false
	}
	cmdCtx, cancel := context.WithTimeout(ctx, p.timeout)
	defer cancel()
	stdout, stderr, err := p.runner.Run(cmdCtx, binaryPath, p.helpArgs...)
	if err != nil {
		return false, false
	}

	text := strings.ToLower(strings.Join(cleanedLines(stdout+"\n"+stderr), "\n"))
	nonInteractive := strings.Contains(text, "non-interactive")
	if p.capabilityTag != "" {
		nonInteractive = nonInteractive || strings.Contains(text, p.capabilityTag)
	}
	supportsJSON := strings.Contains(text, "--json") || strings.Contains(text, "json")
	return nonInteractive, supportsJSON
}
