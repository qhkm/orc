package providers

import (
	"context"
	"fmt"
	"strings"
	"testing"
)

type fakeRunner struct {
	lookPath map[string]string
	outputs  map[string]fakeOutput
}

type fakeOutput struct {
	stdout string
	stderr string
	err    error
}

func (f fakeRunner) LookPath(file string) (string, error) {
	path, ok := f.lookPath[file]
	if !ok {
		return "", fmt.Errorf("not found")
	}
	return path, nil
}

func keyFor(name string, args []string) string {
	return strings.TrimSpace(name + " " + strings.Join(args, " "))
}

func (f fakeRunner) Run(_ context.Context, name string, args ...string) (string, string, error) {
	out, ok := f.outputs[keyFor(name, args)]
	if !ok {
		return "", "", fmt.Errorf("missing command stub")
	}
	return out.stdout, out.stderr, out.err
}

func TestCodexDoctor(t *testing.T) {
	binary := "/usr/local/bin/codex"
	runner := fakeRunner{
		lookPath: map[string]string{codexBinary: binary},
		outputs: map[string]fakeOutput{
			keyFor(binary, []string{"--version"}): {
				stdout: "codex-cli 0.101.0\n",
			},
			keyFor(binary, []string{"exec", "--help"}): {
				stdout: "Run Codex non-interactively\n--json\n",
			},
			keyFor(binary, []string{"login", "status"}): {
				stdout: "Logged in using ChatGPT\n",
			},
		},
	}

	provider := &CodexProvider{runner: runner}
	report := provider.Doctor(context.Background())

	if !report.Installed {
		t.Fatal("expected installed=true")
	}
	if report.Version != "codex-cli 0.101.0" {
		t.Fatalf("unexpected version: %q", report.Version)
	}
	if report.AuthStatus != AuthStatusLoggedIn {
		t.Fatalf("unexpected auth status: %s", report.AuthStatus)
	}
	if !report.SupportsNonInteractive {
		t.Fatal("expected non-interactive support=true")
	}
	if !report.SupportsJSON {
		t.Fatal("expected json support=true")
	}
}
