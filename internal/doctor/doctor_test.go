package doctor

import (
	"context"
	"testing"

	"github.com/drnoraniza/orc/internal/providers"
)

type stubProvider struct {
	name   string
	report providers.DoctorReport
}

func (s stubProvider) Name() string {
	return s.name
}

func (s stubProvider) Doctor(context.Context) providers.DoctorReport {
	return s.report
}

func TestRunFilter(t *testing.T) {
	registry := []providers.Provider{
		stubProvider{name: "codex", report: providers.DoctorReport{Name: "codex", Installed: true}},
		stubProvider{name: "claude", report: providers.DoctorReport{Name: "claude", Installed: true}},
	}

	reports, err := Run(context.Background(), registry, Options{Provider: "codex"})
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if len(reports) != 1 {
		t.Fatalf("expected 1 report, got %d", len(reports))
	}
	if reports[0].Name != "codex" {
		t.Fatalf("expected codex report, got %s", reports[0].Name)
	}
}

func TestRunFilterNotFound(t *testing.T) {
	registry := []providers.Provider{
		stubProvider{name: "codex", report: providers.DoctorReport{Name: "codex", Installed: true}},
	}

	_, err := Run(context.Background(), registry, Options{Provider: "unknown"})
	if err == nil {
		t.Fatal("expected error for missing provider")
	}
}
