package invoke

import (
	"bytes"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestParseCodexJSONL(t *testing.T) {
	t.Parallel()

	input := `{"type":"item.completed","item":{"type":"agent_message","text":"first"}}
{"type":"item.completed","item":{"type":"agent_message","text":"final answer"}}`
	got := parseCodexJSONL(input)
	if got != "final answer" {
		t.Fatalf("unexpected parsed text: %q", got)
	}
}

func TestParseOpenCodeJSONL(t *testing.T) {
	t.Parallel()

	input := `{"type":"text","part":{"text":"draft"}}
{"type":"text","part":{"text":"final opencode answer"}}`
	got := parseOpenCodeJSONL(input)
	if got != "final opencode answer" {
		t.Fatalf("unexpected parsed text: %q", got)
	}
}

func TestParseClaudeJSONL(t *testing.T) {
	t.Parallel()

	input := `{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"hel"}}}
{"type":"assistant","message":{"content":[{"type":"text","text":"hello final"}]}}
{"type":"result","result":"hello final"}`
	got := parseClaudeJSONL(input)
	if got != "hello final" {
		t.Fatalf("unexpected parsed text: %q", got)
	}
}

func TestParseClaudeJSONLDirectDeltas(t *testing.T) {
	t.Parallel()

	input := `{"type":"content_block_delta","delta":{"type":"text_delta","text":"hello "}}
{"type":"content_block_delta","delta":{"type":"text_delta","text":"world"}}`
	got := parseClaudeJSONL(input)
	if got != "hello world" {
		t.Fatalf("unexpected parsed direct deltas: %q", got)
	}
}

func TestNormalizeProvider(t *testing.T) {
	t.Parallel()

	cases := map[string]string{
		"claude":    "claude",
		"anthropic": "claude",
		"codex":     "codex",
		"openai":    "codex",
		"open-code": "opencode",
		"open code": "opencode",
		"moonshot":  "kimi",
	}

	for in, want := range cases {
		if got := normalizeProvider(in); got != want {
			t.Fatalf("normalizeProvider(%q) = %q, want %q", in, got, want)
		}
	}
}

func TestBuildCommandByProvider(t *testing.T) {
	t.Parallel()

	cases := []struct {
		name     string
		provider string
		opts     Options
		binary   string
		contains []string
	}{
		{
			name:     "claude",
			provider: "claude",
			opts:     Options{Message: "hello"},
			binary:   "claude",
			contains: []string{"--dangerously-skip-permissions", "-p", "hello"},
		},
		{
			name:     "codex",
			provider: "codex",
			opts:     Options{Message: "hello"},
			binary:   "codex",
			contains: []string{"exec", "--json", "hello"},
		},
		{
			name:     "opencode",
			provider: "opencode",
			opts:     Options{Message: "hello"},
			binary:   "opencode",
			contains: []string{"run", "--format", "json", "hello"},
		},
		{
			name:     "kimi",
			provider: "kimi",
			opts:     Options{Message: "hello"},
			binary:   "kimi",
			contains: []string{"--print", "--output-format", "text", "--final-message-only", "--prompt", "hello"},
		},
	}

	for _, tc := range cases {
		tc := tc
		t.Run(tc.name, func(t *testing.T) {
			t.Parallel()

			binary, args, err := buildCommand(tc.provider, tc.opts, false)
			if err != nil {
				t.Fatalf("unexpected error: %v", err)
			}
			if binary != tc.binary {
				t.Fatalf("unexpected binary: got %q want %q", binary, tc.binary)
			}

			for _, want := range tc.contains {
				found := false
				for _, got := range args {
					if got == want {
						found = true
						break
					}
				}
				if !found {
					t.Fatalf("expected arg %q in %v", want, args)
				}
			}
		})
	}
}

func TestBuildCommandClaudeStreaming(t *testing.T) {
	t.Parallel()

	_, args, err := buildCommand("claude", Options{Message: "hello"}, true)
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	need := []string{"--verbose", "--output-format", "stream-json", "--include-partial-messages", "-p", "hello"}
	for _, n := range need {
		found := false
		for _, a := range args {
			if a == n {
				found = true
				break
			}
		}
		if !found {
			t.Fatalf("expected %q in args %v", n, args)
		}
	}
}

func TestReadStream(t *testing.T) {
	t.Parallel()

	var sink bytes.Buffer
	chunks := make([]string, 0, 4)
	err := readStream(strings.NewReader("a\nb\nc"), &sink, func(line string) {
		chunks = append(chunks, line)
	})
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if got := sink.String(); got != "a\nb\nc" {
		t.Fatalf("unexpected sink content: %q", got)
	}
	if len(chunks) == 0 {
		t.Fatal("expected at least one streamed chunk")
	}
	if strings.Join(chunks, "") != "a\nb\nc" {
		t.Fatalf("unexpected streamed chunks: %#v", chunks)
	}
}

func TestProviderEnvKimiWithExplicitShareDir(t *testing.T) {
	shareDir := t.TempDir()
	t.Setenv("KIMI_SHARE_DIR", shareDir)

	env, err := providerEnv("kimi", t.TempDir())
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if len(env) != 0 {
		t.Fatalf("expected no override env when KIMI_SHARE_DIR is set and writable, got %v", env)
	}

	if _, err := os.Stat(filepath.Join(shareDir, "logs")); err != nil {
		t.Fatalf("expected logs directory to exist: %v", err)
	}
}

func TestProviderEnvKimiFallbackShareDir(t *testing.T) {
	t.Setenv("KIMI_SHARE_DIR", "")
	t.Setenv("HOME", "/nonexistent-home-for-test")
	workingDir := t.TempDir()

	env, err := providerEnv("kimi", workingDir)
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if len(env) != 1 || !strings.HasPrefix(env[0], "KIMI_SHARE_DIR=") {
		t.Fatalf("unexpected env override: %v", env)
	}
	got := strings.TrimPrefix(env[0], "KIMI_SHARE_DIR=")
	want := filepath.Join(os.TempDir(), "orc", "providers", "kimi-share")
	if got != want {
		t.Fatalf("unexpected fallback share dir: got %s want %s", got, want)
	}
}

func TestKimiNeedsAuth(t *testing.T) {
	t.Parallel()

	if !kimiNeedsAuth("LLM not set", "") {
		t.Fatal("expected auth-needed detection")
	}
	if kimiNeedsAuth("all good", "ok") {
		t.Fatal("unexpected auth-needed detection")
	}
}
