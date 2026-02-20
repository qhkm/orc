package flow

import (
	"os"
	"path/filepath"
	"testing"
	"time"
)

func TestCompileCanonicalPipeline(t *testing.T) {
	t.Parallel()

	def, err := Compile(CompileOptions{
		Name:           "My Team Flow",
		Prompt:         "I want Claude Code to do brainstorming, then use Codex to implement, then cross check with Kimi, then review with Opus.",
		MasterProvider: "codex",
		AvailableProviders: []string{
			"claude", "codex", "kimi", "opencode",
		},
		Now: func() time.Time { return time.Date(2026, 2, 18, 12, 0, 0, 0, time.UTC) },
	})
	if err != nil {
		t.Fatalf("compile failed: %v", err)
	}

	if def.Name != "my_team_flow" {
		t.Fatalf("unexpected flow name: %s", def.Name)
	}
	if len(def.Stages) != 4 {
		t.Fatalf("expected 4 stages, got %d", len(def.Stages))
	}

	expect := []struct {
		intent   string
		provider string
		profile  string
	}{
		{intent: "brainstorm", provider: "claude"},
		{intent: "implement", provider: "codex"},
		{intent: "cross_check", provider: "kimi"},
		{intent: "review", provider: "claude", profile: "opus"},
	}

	for i, want := range expect {
		got := def.Stages[i]
		if got.Intent != want.intent {
			t.Fatalf("stage %d intent: got %s want %s", i, got.Intent, want.intent)
		}
		if got.Provider != want.provider {
			t.Fatalf("stage %d provider: got %s want %s", i, got.Provider, want.provider)
		}
		if got.Profile != want.profile {
			t.Fatalf("stage %d profile: got %s want %s", i, got.Profile, want.profile)
		}
	}

	if def.Stages[1].InputFrom != def.Stages[0].ID {
		t.Fatalf("stage chaining missing between stage 1 and 2")
	}
}

func TestCompileFallbackStage(t *testing.T) {
	t.Parallel()

	def, err := Compile(CompileOptions{
		Prompt:         "Please handle this ticket end to end",
		MasterProvider: "codex",
	})
	if err != nil {
		t.Fatalf("compile failed: %v", err)
	}
	if len(def.Stages) != 1 {
		t.Fatalf("expected fallback single stage, got %d", len(def.Stages))
	}
	if def.Stages[0].Intent != "task" {
		t.Fatalf("unexpected fallback intent: %s", def.Stages[0].Intent)
	}
	if def.Stages[0].Provider != "codex" {
		t.Fatalf("unexpected fallback provider: %s", def.Stages[0].Provider)
	}
}

func TestSaveAndLoad(t *testing.T) {
	t.Parallel()

	tempDir := t.TempDir()
	flowPath := filepath.Join(tempDir, "flows", "demo.json")

	def := Definition{
		Name:           "demo_flow",
		SourcePrompt:   "demo",
		MasterProvider: "codex",
		Stages: []Stage{
			{ID: "brainstorm_1", Intent: "brainstorm", Provider: "claude"},
			{ID: "implement_2", Intent: "implement", Provider: "codex", InputFrom: "brainstorm_1"},
		},
		CreatedAt: time.Now().UTC(),
	}

	if err := Save(flowPath, def); err != nil {
		t.Fatalf("save failed: %v", err)
	}

	loaded, err := Load(flowPath)
	if err != nil {
		t.Fatalf("load failed: %v", err)
	}
	if loaded.Name != def.Name {
		t.Fatalf("unexpected loaded name: %s", loaded.Name)
	}
	if len(loaded.Stages) != 2 {
		t.Fatalf("expected 2 stages, got %d", len(loaded.Stages))
	}
}

func TestLoadMissingFile(t *testing.T) {
	t.Parallel()

	_, err := Load(filepath.Join(t.TempDir(), "missing.json"))
	if err == nil {
		t.Fatal("expected error for missing flow file")
	}
	if !os.IsNotExist(err) && !os.IsNotExist(unwrapError(err)) {
		t.Fatalf("expected not-exist error, got: %v", err)
	}
}

func unwrapError(err error) error {
	type unwrapper interface{ Unwrap() error }
	for {
		u, ok := err.(unwrapper)
		if !ok {
			return err
		}
		next := u.Unwrap()
		if next == nil {
			return err
		}
		err = next
	}
}
