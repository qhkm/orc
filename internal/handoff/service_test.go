package handoff

import (
	"context"
	"os"
	"path/filepath"
	"testing"
	"time"
)

type stubInspector struct {
	state RepoState
	err   error
}

func (s stubInspector) Inspect(context.Context, string) (RepoState, error) {
	return s.state, s.err
}

func TestCreateWritesArtifacts(t *testing.T) {
	t.Parallel()

	tempDir := t.TempDir()
	repoPath := filepath.Join(tempDir, "repo")
	if err := os.MkdirAll(repoPath, 0o755); err != nil {
		t.Fatalf("mkdir repo: %v", err)
	}

	fixedTime := time.Date(2026, 2, 18, 12, 0, 0, 0, time.UTC)
	svc := &Service{
		inspector: stubInspector{state: RepoState{Path: repoPath, Branch: "main", BaseCommit: "abc123", Dirty: true, StatusShort: []string{"M README.md"}}},
		now:       func() time.Time { return fixedTime },
		idGen:     func() string { return "run-001" },
	}

	record, err := svc.Create(context.Background(), CreateOptions{
		From:               "claude",
		To:                 "codex",
		Goal:               "implement auth",
		RepoPath:           repoPath,
		AcceptanceCriteria: []string{"tests pass", "docs updated"},
	})
	if err != nil {
		t.Fatalf("create failed: %v", err)
	}

	if record.RunID != "run-001" {
		t.Fatalf("unexpected run id: %s", record.RunID)
	}
	if _, err := os.Stat(record.HandoffFile); err != nil {
		t.Fatalf("handoff file missing: %v", err)
	}
	if _, err := os.Stat(record.BriefFile); err != nil {
		t.Fatalf("brief file missing: %v", err)
	}

	loaded, err := svc.Load(filepath.Join(repoPath, ".orc", "runs"), record.RunID)
	if err != nil {
		t.Fatalf("load failed: %v", err)
	}
	if loaded.Goal != "implement auth" {
		t.Fatalf("unexpected loaded goal: %s", loaded.Goal)
	}
	if loaded.Repo.Branch != "main" {
		t.Fatalf("unexpected loaded branch: %s", loaded.Repo.Branch)
	}
}

func TestContinueInheritsParent(t *testing.T) {
	t.Parallel()

	tempDir := t.TempDir()
	repoPath := filepath.Join(tempDir, "repo")
	if err := os.MkdirAll(repoPath, 0o755); err != nil {
		t.Fatalf("mkdir repo: %v", err)
	}

	runIDs := []string{"run-001", "run-002"}
	nextID := 0
	svc := &Service{
		inspector: stubInspector{state: RepoState{Path: repoPath, Branch: "main", BaseCommit: "abc123"}},
		now:       time.Now,
		idGen: func() string {
			id := runIDs[nextID]
			nextID++
			return id
		},
	}

	first, err := svc.Create(context.Background(), CreateOptions{
		From:               "claude",
		To:                 "codex",
		Goal:               "fix failing tests",
		RepoPath:           repoPath,
		AcceptanceCriteria: []string{"all tests green"},
	})
	if err != nil {
		t.Fatalf("create failed: %v", err)
	}

	second, err := svc.Continue(context.Background(), ContinueOptions{
		RunID:    first.RunID,
		To:       "opencode",
		RepoPath: repoPath,
	})
	if err != nil {
		t.Fatalf("continue failed: %v", err)
	}

	if second.RunID != "run-002" {
		t.Fatalf("unexpected continued run id: %s", second.RunID)
	}
	if second.SourceAgent != "codex" {
		t.Fatalf("continue should use parent target as source, got %s", second.SourceAgent)
	}
	if second.PreviousRunID != first.RunID {
		t.Fatalf("unexpected previous run id: %s", second.PreviousRunID)
	}
	if second.Goal != first.Goal {
		t.Fatalf("expected goal inheritance, got %s", second.Goal)
	}
	if len(second.AcceptanceCriteria) != 1 || second.AcceptanceCriteria[0] != "all tests green" {
		t.Fatalf("acceptance criteria not inherited: %#v", second.AcceptanceCriteria)
	}
}
