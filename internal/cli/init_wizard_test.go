package cli

import (
	"testing"

	"github.com/drnoraniza/orc/internal/config"
)

func TestParseProviderSelectionByIndex(t *testing.T) {
	t.Parallel()

	detected := []config.Provider{
		{Name: "codex"},
		{Name: "claude"},
		{Name: "kimi"},
	}

	selected, err := parseProviderSelection("1,3", detected)
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if len(selected) != 2 {
		t.Fatalf("expected 2 providers, got %d", len(selected))
	}
	if selected[0].Name != "codex" || selected[1].Name != "kimi" {
		t.Fatalf("unexpected selection order: %#v", selected)
	}
}

func TestParseProviderSelectionByName(t *testing.T) {
	t.Parallel()

	detected := []config.Provider{
		{Name: "codex"},
		{Name: "claude"},
		{Name: "opencode"},
	}

	selected, err := parseProviderSelection("claude, opencode", detected)
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if len(selected) != 2 {
		t.Fatalf("expected 2 providers, got %d", len(selected))
	}
	if selected[0].Name != "claude" || selected[1].Name != "opencode" {
		t.Fatalf("unexpected provider names: %#v", selected)
	}
}

func TestParseProviderSelectionInvalid(t *testing.T) {
	t.Parallel()

	detected := []config.Provider{{Name: "codex"}}
	_, err := parseProviderSelection("5", detected)
	if err == nil {
		t.Fatal("expected selection error")
	}
}
