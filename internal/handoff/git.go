package handoff

import (
	"bytes"
	"context"
	"fmt"
	"os/exec"
	"strings"
)

type RepoInspector interface {
	Inspect(ctx context.Context, repoPath string) (RepoState, error)
}

type GitInspector struct{}

func NewGitInspector() RepoInspector {
	return GitInspector{}
}

func (GitInspector) Inspect(ctx context.Context, repoPath string) (RepoState, error) {
	branch, err := runGit(ctx, repoPath, "rev-parse", "--abbrev-ref", "HEAD")
	if err != nil {
		return RepoState{}, err
	}
	commit, err := runGit(ctx, repoPath, "rev-parse", "HEAD")
	if err != nil {
		return RepoState{}, err
	}
	statusText, err := runGit(ctx, repoPath, "status", "--short")
	if err != nil {
		return RepoState{}, err
	}

	statusLines := make([]string, 0)
	for _, line := range strings.Split(statusText, "\n") {
		trimmed := strings.TrimSpace(line)
		if trimmed == "" {
			continue
		}
		statusLines = append(statusLines, trimmed)
	}

	return RepoState{
		Path:        repoPath,
		Branch:      branch,
		BaseCommit:  commit,
		Dirty:       len(statusLines) > 0,
		StatusShort: statusLines,
	}, nil
}

func runGit(ctx context.Context, repoPath string, args ...string) (string, error) {
	cmdArgs := append([]string{"-C", repoPath}, args...)
	cmd := exec.CommandContext(ctx, "git", cmdArgs...)
	var stdout bytes.Buffer
	var stderr bytes.Buffer
	cmd.Stdout = &stdout
	cmd.Stderr = &stderr
	if err := cmd.Run(); err != nil {
		return "", fmt.Errorf("git %s failed: %w (%s)", strings.Join(args, " "), err, strings.TrimSpace(stderr.String()))
	}
	return strings.TrimSpace(stdout.String()), nil
}
