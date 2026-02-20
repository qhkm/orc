package handoff

import (
	"context"
	"crypto/rand"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"time"
)

const (
	handoffFileName = "handoff.json"
	briefFileName   = "brief.md"
)

type Service struct {
	inspector RepoInspector
	now       func() time.Time
	idGen     func() string
}

type CreateOptions struct {
	From               string
	To                 string
	Goal               string
	RepoPath           string
	RunsRoot           string
	AcceptanceCriteria []string
	PreviousRunID      string
}

type ContinueOptions struct {
	RunID              string
	To                 string
	Goal               string
	RepoPath           string
	RunsRoot           string
	AcceptanceCriteria []string
}

func NewService() *Service {
	return &Service{
		inspector: NewGitInspector(),
		now:       time.Now,
		idGen:     defaultRunID,
	}
}

func (s *Service) Create(ctx context.Context, opts CreateOptions) (RunRecord, error) {
	if strings.TrimSpace(opts.From) == "" {
		return RunRecord{}, fmt.Errorf("--from is required")
	}
	if strings.TrimSpace(opts.To) == "" {
		return RunRecord{}, fmt.Errorf("--to is required")
	}
	if strings.TrimSpace(opts.Goal) == "" {
		return RunRecord{}, fmt.Errorf("--goal is required")
	}

	repoPath, err := resolveRepoPath(opts.RepoPath)
	if err != nil {
		return RunRecord{}, err
	}
	runsRoot := resolveRunsRootForCreate(repoPath, opts.RunsRoot)
	runID := s.idGen()

	runDir := filepath.Join(runsRoot, runID)
	if _, err := os.Stat(runDir); err == nil {
		return RunRecord{}, fmt.Errorf("run id collision: %s", runID)
	}
	if err := os.MkdirAll(runDir, 0o755); err != nil {
		return RunRecord{}, fmt.Errorf("create run directory: %w", err)
	}

	timestamp := s.now().UTC()
	repoState, inspectErr := s.inspector.Inspect(ctx, repoPath)
	warnings := make([]string, 0)
	if inspectErr != nil {
		repoState = RepoState{Path: repoPath}
		warnings = append(warnings, fmt.Sprintf("repo inspection failed: %v", inspectErr))
	}
	if repoState.Path == "" {
		repoState.Path = repoPath
	}

	record := RunRecord{
		RunID:              runID,
		SourceAgent:        strings.TrimSpace(opts.From),
		TargetAgent:        strings.TrimSpace(opts.To),
		Goal:               strings.TrimSpace(opts.Goal),
		AcceptanceCriteria: cloneStrings(opts.AcceptanceCriteria),
		Repo:               repoState,
		PreviousRunID:      strings.TrimSpace(opts.PreviousRunID),
		Status:             "created",
		CreatedAt:          timestamp,
		UpdatedAt:          timestamp,
		RunDir:             runDir,
		HandoffFile:        filepath.Join(runDir, handoffFileName),
		BriefFile:          filepath.Join(runDir, briefFileName),
		Warnings:           warnings,
	}

	if err := writeRunRecord(record); err != nil {
		return RunRecord{}, err
	}
	return record, nil
}

func (s *Service) Continue(ctx context.Context, opts ContinueOptions) (RunRecord, error) {
	if strings.TrimSpace(opts.RunID) == "" {
		return RunRecord{}, fmt.Errorf("run id is required")
	}
	if strings.TrimSpace(opts.To) == "" {
		return RunRecord{}, fmt.Errorf("--to is required")
	}

	lookupRoot := strings.TrimSpace(opts.RunsRoot)
	if lookupRoot == "" && strings.TrimSpace(opts.RepoPath) != "" {
		repoPath, err := resolveRepoPath(opts.RepoPath)
		if err != nil {
			return RunRecord{}, err
		}
		lookupRoot = filepath.Join(repoPath, ".orc", "runs")
	}

	parent, err := s.Load(lookupRoot, opts.RunID)
	if err != nil {
		return RunRecord{}, err
	}

	goal := strings.TrimSpace(opts.Goal)
	if goal == "" {
		goal = parent.Goal
	}
	acceptance := cloneStrings(opts.AcceptanceCriteria)
	if len(acceptance) == 0 {
		acceptance = cloneStrings(parent.AcceptanceCriteria)
	}
	repoPath := strings.TrimSpace(opts.RepoPath)
	if repoPath == "" {
		repoPath = parent.Repo.Path
	}
	from := strings.TrimSpace(parent.TargetAgent)
	if from == "" {
		from = parent.SourceAgent
	}

	createRoot := strings.TrimSpace(opts.RunsRoot)
	if createRoot == "" {
		createRoot = lookupRoot
	}

	return s.Create(ctx, CreateOptions{
		From:               from,
		To:                 strings.TrimSpace(opts.To),
		Goal:               goal,
		RepoPath:           repoPath,
		RunsRoot:           createRoot,
		AcceptanceCriteria: acceptance,
		PreviousRunID:      parent.RunID,
	})
}

func (s *Service) Load(runsRoot, runID string) (RunRecord, error) {
	if strings.TrimSpace(runID) == "" {
		return RunRecord{}, fmt.Errorf("run id is required")
	}

	resolvedRoot, err := resolveRunsRootForLookup(runsRoot)
	if err != nil {
		return RunRecord{}, err
	}

	handoffFile := filepath.Join(resolvedRoot, runID, handoffFileName)
	data, err := os.ReadFile(handoffFile)
	if err != nil {
		return RunRecord{}, fmt.Errorf("read handoff file: %w", err)
	}

	var record RunRecord
	if err := json.Unmarshal(data, &record); err != nil {
		return RunRecord{}, fmt.Errorf("parse handoff file: %w", err)
	}
	if record.RunID == "" {
		record.RunID = runID
	}
	if record.RunDir == "" {
		record.RunDir = filepath.Dir(handoffFile)
	}
	if record.HandoffFile == "" {
		record.HandoffFile = handoffFile
	}
	if record.BriefFile == "" {
		record.BriefFile = filepath.Join(record.RunDir, briefFileName)
	}
	return record, nil
}

func writeRunRecord(record RunRecord) error {
	jsonData, err := json.MarshalIndent(record, "", "  ")
	if err != nil {
		return fmt.Errorf("marshal run record: %w", err)
	}
	if err := os.WriteFile(record.HandoffFile, jsonData, 0o644); err != nil {
		return fmt.Errorf("write handoff file: %w", err)
	}

	brief := renderBrief(record)
	if err := os.WriteFile(record.BriefFile, []byte(brief), 0o644); err != nil {
		return fmt.Errorf("write brief file: %w", err)
	}
	return nil
}

func renderBrief(record RunRecord) string {
	builder := &strings.Builder{}
	builder.WriteString("# Agent Handoff Brief\n\n")
	builder.WriteString(fmt.Sprintf("- Run ID: `%s`\n", record.RunID))
	builder.WriteString(fmt.Sprintf("- Flow: `%s -> %s`\n", record.SourceAgent, record.TargetAgent))
	builder.WriteString(fmt.Sprintf("- Goal: %s\n", record.Goal))
	builder.WriteString(fmt.Sprintf("- Repo: `%s`\n", record.Repo.Path))
	if record.Repo.Branch != "" {
		builder.WriteString(fmt.Sprintf("- Branch: `%s`\n", record.Repo.Branch))
	}
	if record.Repo.BaseCommit != "" {
		builder.WriteString(fmt.Sprintf("- Base Commit: `%s`\n", record.Repo.BaseCommit))
	}
	builder.WriteString(fmt.Sprintf("- Dirty Working Tree: `%t`\n", record.Repo.Dirty))
	if record.PreviousRunID != "" {
		builder.WriteString(fmt.Sprintf("- Previous Run: `%s`\n", record.PreviousRunID))
	}

	if len(record.AcceptanceCriteria) > 0 {
		builder.WriteString("\n## Acceptance Criteria\n\n")
		for _, criterion := range record.AcceptanceCriteria {
			builder.WriteString(fmt.Sprintf("- %s\n", criterion))
		}
	}

	if len(record.Repo.StatusShort) > 0 {
		builder.WriteString("\n## Git Status (short)\n\n")
		for _, line := range record.Repo.StatusShort {
			builder.WriteString(fmt.Sprintf("- `%s`\n", line))
		}
	}

	if len(record.Warnings) > 0 {
		builder.WriteString("\n## Warnings\n\n")
		for _, warning := range record.Warnings {
			builder.WriteString(fmt.Sprintf("- %s\n", warning))
		}
	}

	builder.WriteString("\n## Suggested Prompt\n\n")
	builder.WriteString("Use this handoff context, continue the task, and return: summary, patch, tests run, and open questions.\n")
	return builder.String()
}

func cloneStrings(values []string) []string {
	if len(values) == 0 {
		return nil
	}
	out := make([]string, 0, len(values))
	for _, value := range values {
		trimmed := strings.TrimSpace(value)
		if trimmed == "" {
			continue
		}
		out = append(out, trimmed)
	}
	return out
}

func resolveRepoPath(repoPath string) (string, error) {
	candidate := strings.TrimSpace(repoPath)
	if candidate == "" {
		cwd, err := os.Getwd()
		if err != nil {
			return "", fmt.Errorf("resolve repo path: %w", err)
		}
		return filepath.Clean(cwd), nil
	}

	absolute, err := filepath.Abs(candidate)
	if err != nil {
		return "", fmt.Errorf("resolve repo path: %w", err)
	}
	return filepath.Clean(absolute), nil
}

func resolveRunsRootForCreate(repoPath, runsRoot string) string {
	trimmed := strings.TrimSpace(runsRoot)
	if trimmed == "" {
		return filepath.Join(repoPath, ".orc", "runs")
	}
	if filepath.IsAbs(trimmed) {
		return filepath.Clean(trimmed)
	}
	return filepath.Join(repoPath, trimmed)
}

func resolveRunsRootForLookup(runsRoot string) (string, error) {
	trimmed := strings.TrimSpace(runsRoot)
	if trimmed == "" {
		cwd, err := os.Getwd()
		if err != nil {
			return "", fmt.Errorf("resolve runs root: %w", err)
		}
		return filepath.Join(cwd, ".orc", "runs"), nil
	}
	if filepath.IsAbs(trimmed) {
		return filepath.Clean(trimmed), nil
	}
	cwd, err := os.Getwd()
	if err != nil {
		return "", fmt.Errorf("resolve runs root: %w", err)
	}
	return filepath.Join(cwd, trimmed), nil
}

func defaultRunID() string {
	now := time.Now().UTC()
	randBytes := make([]byte, 3)
	if _, err := rand.Read(randBytes); err != nil {
		return fmt.Sprintf("%s-%d", now.Format("20060102T150405Z"), now.UnixNano())
	}
	return fmt.Sprintf("%s-%x", now.Format("20060102T150405Z"), randBytes)
}
