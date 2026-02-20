package cli

import (
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/drnoraniza/orc/internal/flow"
)

func TestResolveChatFlowPathPrefersNamedFlowWhenPresent(t *testing.T) {
	t.Parallel()

	repo := t.TempDir()
	flowsDir := filepath.Join(repo, ".orc", "flows")
	if err := os.MkdirAll(flowsDir, 0o755); err != nil {
		t.Fatalf("create flows dir: %v", err)
	}
	if err := os.WriteFile(filepath.Join(flowsDir, "team_flow.json"), []byte("{}"), 0o644); err != nil {
		t.Fatalf("write flow: %v", err)
	}

	path, err := resolveChatFlowPath(repo, "Team Flow", "")
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	want := filepath.Join(repo, ".orc", "flows", "team_flow.json")
	if path != want {
		t.Fatalf("unexpected flow path: got %s want %s", path, want)
	}
}

func TestResolveChatFlowPathFallbackToFirstFlow(t *testing.T) {
	t.Parallel()

	repo := t.TempDir()
	flowsDir := filepath.Join(repo, ".orc", "flows")
	if err := os.MkdirAll(flowsDir, 0o755); err != nil {
		t.Fatalf("create flows dir: %v", err)
	}
	if err := os.WriteFile(filepath.Join(flowsDir, "z_flow.json"), []byte("{}"), 0o644); err != nil {
		t.Fatalf("write flow: %v", err)
	}
	if err := os.WriteFile(filepath.Join(flowsDir, "a_flow.json"), []byte("{}"), 0o644); err != nil {
		t.Fatalf("write flow: %v", err)
	}

	path, err := resolveChatFlowPath(repo, "default-flow", "")
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	want := filepath.Join(repo, ".orc", "flows", "a_flow.json")
	if path != want {
		t.Fatalf("unexpected fallback flow path: got %s want %s", path, want)
	}
}

func TestResolveChatFlowPathMissing(t *testing.T) {
	t.Parallel()

	_, err := resolveChatFlowPath(t.TempDir(), "default-flow", "")
	if err == nil {
		t.Fatal("expected error when no flow exists")
	}
}

func TestStageModelClaudeProfile(t *testing.T) {
	t.Parallel()

	if got := stageModel("claude", "opus"); got != "claude-opus-4-6" {
		t.Fatalf("unexpected model mapping: %s", got)
	}
	if got := stageModel("claude", "sonnet"); got != "claude-sonnet-4-5" {
		t.Fatalf("unexpected model mapping: %s", got)
	}
}

func TestIsLikelyCodingOrRepoRequest(t *testing.T) {
	t.Parallel()

	cases := []struct {
		message string
		want    bool
	}{
		{message: "hey", want: false},
		{message: "hello", want: false},
		{message: "fix auth middleware and add tests", want: true},
		{message: "review src/main.go for bugs", want: true},
		{message: "what's the weather today", want: false},
	}

	for _, tc := range cases {
		if got := isLikelyCodingOrRepoRequest(tc.message); got != tc.want {
			t.Fatalf("isLikelyCodingOrRepoRequest(%q) = %t, want %t", tc.message, got, tc.want)
		}
	}
}

func TestParseScopeGuardDecision(t *testing.T) {
	t.Parallel()

	decision, reply := parseScopeGuardDecision(`{"decision":"out_of_scope","reply":"coding only"}`)
	if decision != "out_of_scope" || reply != "coding only" {
		t.Fatalf("unexpected parse result: decision=%q reply=%q", decision, reply)
	}

	decision, reply = parseScopeGuardDecision("out_of_scope: this is not coding related")
	if decision != "out_of_scope" {
		t.Fatalf("expected out_of_scope fallback, got %q", decision)
	}
	if reply == "" {
		t.Fatal("expected fallback reply text")
	}
}

func TestIsSupportedFollowUpProvider(t *testing.T) {
	t.Parallel()

	if !isSupportedFollowUpProvider("codex") {
		t.Fatal("expected codex to be supported")
	}
	if isSupportedFollowUpProvider("unknown") {
		t.Fatal("expected unknown provider to be rejected")
	}
}

func TestDefaultProviderForIntent(t *testing.T) {
	t.Parallel()

	runtime := chatRuntime{
		Flow: flow.Definition{
			Stages: []flow.Stage{
				{Intent: "brainstorm", Provider: "claude"},
				{Intent: "cross_check", Provider: "kimi"},
			},
		},
	}

	if got := runtime.defaultProviderForIntent("cross_check"); got != "kimi" {
		t.Fatalf("unexpected provider for cross_check: %s", got)
	}
	if got := runtime.defaultProviderForIntent("review"); got != "codex" {
		t.Fatalf("unexpected provider for review: %s", got)
	}
}

func TestDecodeCodexStreamLine(t *testing.T) {
	t.Parallel()

	lines := decodeProviderStreamLine("codex", `{"type":"item.completed","item":{"type":"agent_message","text":"first\nsecond"}}`, false)
	if len(lines) != 2 || lines[0] != "first" || lines[1] != "second" {
		t.Fatalf("unexpected codex decode lines: %#v", lines)
	}

	ignored := decodeProviderStreamLine("codex", `{"type":"thread.started","thread_id":"abc"}`, false)
	if len(ignored) != 0 {
		t.Fatalf("expected thread.started to be ignored, got %#v", ignored)
	}
}

func TestDecodeOpenCodeStreamLine(t *testing.T) {
	t.Parallel()

	lines := decodeProviderStreamLine("opencode", `{"type":"text","part":{"text":"hello from opencode"}}`, false)
	if len(lines) != 1 || lines[0] != "hello from opencode" {
		t.Fatalf("unexpected opencode decode lines: %#v", lines)
	}
}

func TestDecodeClaudeStreamLine(t *testing.T) {
	t.Parallel()

	delta := `{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"hello"}}}`
	lines := decodeProviderStreamLine("claude", delta, false)
	if len(lines) != 1 || lines[0] != "hello" {
		t.Fatalf("unexpected claude delta decode: %#v", lines)
	}

	assistant := `{"type":"assistant","message":{"content":[{"type":"text","text":"final answer"}]}}`
	lines = decodeProviderStreamLine("claude", assistant, false)
	if len(lines) != 0 {
		t.Fatalf("expected assistant to be suppressed in non-verbose stream, got %#v", lines)
	}

	lines = decodeProviderStreamLine("claude", assistant, true)
	if len(lines) != 1 || lines[0] != "final answer" {
		t.Fatalf("unexpected claude assistant decode in verbose mode: %#v", lines)
	}

	direct := `{"type":"content_block_delta","delta":{"type":"text_delta","text":"from direct delta"}}`
	lines = decodeProviderStreamLine("claude", direct, false)
	if len(lines) != 1 || lines[0] != "from direct delta" {
		t.Fatalf("unexpected claude direct decode: %#v", lines)
	}

	withLeadingSpace := `{"type":"content_block_delta","delta":{"type":"text_delta","text":" to understand"}}`
	lines = decodeProviderStreamLine("claude", withLeadingSpace, false)
	if len(lines) != 1 || lines[0] != " to understand" {
		t.Fatalf("expected leading space to be preserved, got %#v", lines)
	}

	plain := "plain text fallback"
	lines = decodeProviderStreamLine("claude", plain, false)
	if len(lines) != 1 || lines[0] != plain {
		t.Fatalf("unexpected claude plain-text fallback: %#v", lines)
	}
}

func TestIsIgnorableStreamStderr(t *testing.T) {
	t.Parallel()

	if !isIgnorableStreamStderr("codex", "ERROR codex_core::rollout::list: state db missing rollout path") {
		t.Fatal("expected codex rollout noise to be ignored")
	}
	if isIgnorableStreamStderr("codex", "fatal: authentication failed") {
		t.Fatal("unexpected ignore for real stderr")
	}
}

func TestFormatCommandForDisplay(t *testing.T) {
	t.Parallel()

	cmd := []string{"claude", "--flag", "value", strings.Repeat("x", 200)}
	got := formatCommandForDisplay(cmd, 0)
	if !strings.Contains(got, "<prompt:200 chars>") {
		t.Fatalf("expected prompt placeholder in command, got %q", got)
	}
}

func TestWrapForPanel(t *testing.T) {
	t.Parallel()

	lines := wrapForPanel("this is a long line for wrapping", 10)
	if len(lines) < 2 {
		t.Fatalf("expected wrapped lines, got %#v", lines)
	}
}

func TestChooseInlineFlushCut(t *testing.T) {
	t.Parallel()

	if got := chooseInlineFlushCut("This is one sentence. Then another", 10, 220); got <= 0 {
		t.Fatalf("expected sentence boundary cut, got %d", got)
	}
	if got := chooseInlineFlushCut("word1 word2 word3", 5, 220); got <= 0 {
		t.Fatalf("expected word boundary cut, got %d", got)
	}
	if got := chooseInlineFlushCut(strings.Repeat("x", 130), 10, 220); got != 130 {
		t.Fatalf("expected hard cut for long chunk, got %d", got)
	}
}
