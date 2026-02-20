package invoke

import (
	"bufio"
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"sync"
	"time"
)

type Options struct {
	Provider        string
	Model           string
	Message         string
	WorkingDir      string
	ContinueSession bool
	Timeout         time.Duration
}

type Result struct {
	Provider   string   `json:"provider"`
	WorkingDir string   `json:"working_dir"`
	Command    []string `json:"command"`
	DurationMs int64    `json:"duration_ms"`
	RawOutput  string   `json:"raw_output"`
	Response   string   `json:"response"`
}

func Execute(ctx context.Context, opts Options) (Result, error) {
	return execute(ctx, opts, nil, nil, false)
}

func ExecuteStream(ctx context.Context, opts Options, onStdout func(string), onStderr func(string)) (Result, error) {
	return execute(ctx, opts, onStdout, onStderr, true)
}

func execute(ctx context.Context, opts Options, onStdout func(string), onStderr func(string), streaming bool) (Result, error) {
	provider := normalizeProvider(opts.Provider)
	if provider == "" {
		return Result{}, fmt.Errorf("provider is required")
	}
	if strings.TrimSpace(opts.Message) == "" {
		return Result{}, fmt.Errorf("message is required")
	}
	if strings.TrimSpace(opts.WorkingDir) == "" {
		return Result{}, fmt.Errorf("working directory is required")
	}
	if opts.Timeout <= 0 {
		opts.Timeout = 120 * time.Second
	}

	workingDir, err := filepath.Abs(opts.WorkingDir)
	if err != nil {
		return Result{}, fmt.Errorf("resolve working directory: %w", err)
	}

	envOverrides, err := providerEnv(provider, workingDir)
	if err != nil {
		return Result{}, err
	}

	binary, args, err := buildCommand(provider, opts, streaming)
	if err != nil {
		return Result{}, err
	}

	cmdCtx, cancel := context.WithTimeout(ctx, opts.Timeout)
	defer cancel()

	command := exec.CommandContext(cmdCtx, binary, args...)
	command.Dir = workingDir
	if len(envOverrides) > 0 {
		command.Env = append(os.Environ(), envOverrides...)
	}
	stdoutPipe, err := command.StdoutPipe()
	if err != nil {
		return Result{}, fmt.Errorf("capture stdout: %w", err)
	}
	stderrPipe, err := command.StderrPipe()
	if err != nil {
		return Result{}, fmt.Errorf("capture stderr: %w", err)
	}

	started := time.Now()
	if err := command.Start(); err != nil {
		return Result{}, fmt.Errorf("start %s command: %w", provider, err)
	}

	var stdout bytes.Buffer
	var stderr bytes.Buffer
	var firstReadErr error
	var readErrMu sync.Mutex
	setReadErr := func(err error) {
		if err == nil {
			return
		}
		readErrMu.Lock()
		defer readErrMu.Unlock()
		if firstReadErr == nil {
			firstReadErr = err
		}
	}

	var wg sync.WaitGroup
	wg.Add(2)
	go func() {
		defer wg.Done()
		setReadErr(readStream(stdoutPipe, &stdout, onStdout))
	}()
	go func() {
		defer wg.Done()
		setReadErr(readStream(stderrPipe, &stderr, onStderr))
	}()

	runErr := command.Wait()
	_ = stdoutPipe.Close()
	_ = stderrPipe.Close()
	waitDone := make(chan struct{})
	go func() {
		wg.Wait()
		close(waitDone)
	}()
	select {
	case <-waitDone:
	case <-time.After(2 * time.Second):
		return Result{}, fmt.Errorf("%s output stream did not close after process exit", provider)
	}
	duration := time.Since(started)

	if firstReadErr != nil {
		return Result{}, fmt.Errorf("%s output read failed: %w", provider, firstReadErr)
	}

	rawStdout := strings.TrimSpace(stdout.String())
	rawStderr := strings.TrimSpace(stderr.String())
	rawOutput := rawStdout
	if rawOutput == "" {
		rawOutput = rawStderr
	} else if rawStderr != "" {
		rawOutput = rawOutput + "\n" + rawStderr
	}

	if runErr != nil {
		if cmdCtx.Err() == context.DeadlineExceeded {
			return Result{}, fmt.Errorf("%s command timed out after %s", provider, opts.Timeout)
		}
		if rawStderr != "" {
			return Result{}, fmt.Errorf("%s command failed: %s", provider, rawStderr)
		}
		return Result{}, fmt.Errorf("%s command failed: %w", provider, runErr)
	}

	response := rawStdout
	switch provider {
	case "claude":
		response = parseClaudeJSONL(rawStdout)
	case "codex":
		response = parseCodexJSONL(rawStdout)
	case "opencode":
		response = parseOpenCodeJSONL(rawStdout)
	}
	if strings.TrimSpace(response) == "" {
		response = strings.TrimSpace(rawOutput)
	}
	if provider == "kimi" && kimiNeedsAuth(response, rawOutput) {
		return Result{}, fmt.Errorf("kimi is not authenticated/configured (LLM not set). Run `kimi` once to sign in, or set KIMI_API_KEY")
	}

	return Result{
		Provider:   provider,
		WorkingDir: workingDir,
		Command:    append([]string{binary}, args...),
		DurationMs: duration.Milliseconds(),
		RawOutput:  rawOutput,
		Response:   response,
	}, nil
}

func providerEnv(provider, workingDir string) ([]string, error) {
	if provider != "kimi" {
		return nil, nil
	}

	explicitShare := strings.TrimSpace(os.Getenv("KIMI_SHARE_DIR"))
	if explicitShare != "" {
		if err := ensureKimiShareDirWritable(explicitShare); err != nil {
			return nil, fmt.Errorf("KIMI_SHARE_DIR is not writable: %w", err)
		}
		return nil, nil
	}

	homeDir, err := os.UserHomeDir()
	if err == nil {
		defaultShare := filepath.Join(homeDir, ".kimi")
		if err := ensureKimiShareDirWritable(defaultShare); err == nil {
			return nil, nil
		}
	}

	fallbackShare := filepath.Join(os.TempDir(), "orc", "providers", "kimi-share")
	if err := ensureKimiShareDirWritable(fallbackShare); err != nil {
		return nil, fmt.Errorf("cannot prepare fallback Kimi share directory %s: %w", fallbackShare, err)
	}
	return []string{"KIMI_SHARE_DIR=" + fallbackShare}, nil
}

func ensureKimiShareDirWritable(shareDir string) error {
	shareDir = strings.TrimSpace(shareDir)
	if shareDir == "" {
		return fmt.Errorf("empty share directory")
	}

	logsDir := filepath.Join(shareDir, "logs")
	if err := os.MkdirAll(logsDir, 0o755); err != nil {
		return err
	}

	probeFile := filepath.Join(logsDir, ".orc-write-check")
	file, err := os.OpenFile(probeFile, os.O_CREATE|os.O_WRONLY|os.O_APPEND, 0o644)
	if err != nil {
		return err
	}
	_ = file.Close()
	_ = os.Remove(probeFile)
	return nil
}

func kimiNeedsAuth(response, rawOutput string) bool {
	combined := strings.ToLower(strings.TrimSpace(response + "\n" + rawOutput))
	return strings.Contains(combined, "llm not set")
}

func readStream(reader io.Reader, sink *bytes.Buffer, onChunk func(string)) error {
	br := bufio.NewReader(reader)
	buffer := make([]byte, 4096)
	for {
		n, err := br.Read(buffer)
		if n > 0 {
			chunk := string(buffer[:n])
			_, _ = sink.WriteString(chunk)
			if onChunk != nil {
				onChunk(chunk)
			}
		}
		if err != nil {
			if errors.Is(err, io.EOF) || errors.Is(err, os.ErrClosed) || strings.Contains(strings.ToLower(err.Error()), "file already closed") {
				return nil
			}
			return err
		}
	}
}

func buildCommand(provider string, opts Options, streaming bool) (string, []string, error) {
	model := strings.TrimSpace(opts.Model)
	message := strings.TrimSpace(opts.Message)

	switch provider {
	case "claude":
		args := []string{"--dangerously-skip-permissions"}
		if model != "" {
			args = append(args, "--model", model)
		}
		if opts.ContinueSession {
			args = append(args, "-c")
		}
		if streaming {
			// Claude only emits incremental chunks in stream-json mode.
			args = append(args, "--verbose", "--output-format", "stream-json", "--include-partial-messages")
		}
		args = append(args, "-p", message)
		return "claude", args, nil
	case "codex":
		args := []string{"exec"}
		if opts.ContinueSession {
			args = append(args, "resume", "--last")
		}
		if model != "" {
			args = append(args, "--model", model)
		}
		args = append(args, "--skip-git-repo-check", "--dangerously-bypass-approvals-and-sandbox", "--json", message)
		return "codex", args, nil
	case "opencode":
		args := []string{"run", "--format", "json"}
		if model != "" {
			args = append(args, "--model", model)
		}
		if opts.ContinueSession {
			args = append(args, "-c")
		}
		args = append(args, message)
		return "opencode", args, nil
	case "kimi":
		args := []string{"--print", "--output-format", "text", "--final-message-only"}
		if model != "" {
			args = append(args, "--model", model)
		}
		if opts.ContinueSession {
			args = append(args, "--continue")
		}
		args = append(args, "--prompt", message)
		return "kimi", args, nil
	default:
		return "", nil, fmt.Errorf("unsupported provider %q (expected claude, codex, opencode, or kimi)", provider)
	}
}

func parseClaudeJSONL(output string) string {
	best := ""
	var deltaBuilder strings.Builder
	for _, line := range strings.Split(output, "\n") {
		trimmed := strings.TrimSpace(line)
		if trimmed == "" {
			continue
		}
		var event map[string]interface{}
		if err := json.Unmarshal([]byte(trimmed), &event); err != nil {
			if !strings.HasPrefix(trimmed, "{") {
				if best != "" {
					best += "\n"
				}
				best += trimmed
			}
			continue
		}

		if final := extractClaudeFinalText(event); strings.TrimSpace(final) != "" {
			best = final
			continue
		}

		if delta := extractClaudeDeltaText(event); strings.TrimSpace(delta) != "" {
			deltaBuilder.WriteString(delta)
		}
	}
	if strings.TrimSpace(best) == "" && deltaBuilder.Len() > 0 {
		best = deltaBuilder.String()
	}
	return strings.TrimSpace(best)
}

func extractClaudeFinalText(event map[string]interface{}) string {
	if nested, ok := event["event"].(map[string]interface{}); ok {
		if text := extractClaudeFinalText(nested); strings.TrimSpace(text) != "" {
			return text
		}
	}

	if result, _ := event["result"].(string); strings.TrimSpace(result) != "" {
		return result
	}

	if message, ok := event["message"].(map[string]interface{}); ok {
		if text := extractClaudeMessageText(message); strings.TrimSpace(text) != "" {
			return text
		}
	}

	if content, ok := event["content"].([]interface{}); ok {
		if text := extractClaudeContentText(content); strings.TrimSpace(text) != "" {
			return text
		}
	}

	if text, _ := event["text"].(string); strings.TrimSpace(text) != "" {
		return text
	}
	return ""
}

func extractClaudeDeltaText(event map[string]interface{}) string {
	if nested, ok := event["event"].(map[string]interface{}); ok {
		if text := extractClaudeDeltaText(nested); strings.TrimSpace(text) != "" {
			return text
		}
	}
	delta, ok := event["delta"].(map[string]interface{})
	if !ok {
		return ""
	}
	text, _ := delta["text"].(string)
	if strings.TrimSpace(text) == "" {
		return ""
	}
	return text
}

func extractClaudeMessageText(message map[string]interface{}) string {
	if content, ok := message["content"].([]interface{}); ok {
		if text := extractClaudeContentText(content); strings.TrimSpace(text) != "" {
			return text
		}
	}
	if text, _ := message["text"].(string); strings.TrimSpace(text) != "" {
		return text
	}
	return ""
}

func extractClaudeContentText(content []interface{}) string {
	texts := make([]string, 0, len(content))
	for _, block := range content {
		blockMap, ok := block.(map[string]interface{})
		if !ok {
			continue
		}
		if blockType, _ := blockMap["type"].(string); blockType != "" && blockType != "text" {
			continue
		}
		if text, _ := blockMap["text"].(string); strings.TrimSpace(text) != "" {
			texts = append(texts, strings.TrimSpace(text))
		}
	}
	return strings.TrimSpace(strings.Join(texts, "\n"))
}

func normalizeProvider(value string) string {
	normalized := strings.TrimSpace(strings.ToLower(value))
	switch normalized {
	case "anthropic":
		return "claude"
	case "openai":
		return "codex"
	case "open code", "open-code":
		return "opencode"
	case "moonshot":
		return "kimi"
	default:
		return normalized
	}
}

func parseCodexJSONL(output string) string {
	best := ""
	for _, line := range strings.Split(output, "\n") {
		trimmed := strings.TrimSpace(line)
		if trimmed == "" {
			continue
		}

		var event map[string]interface{}
		if err := json.Unmarshal([]byte(trimmed), &event); err != nil {
			continue
		}

		if text := extractCodexText(event); text != "" {
			best = text
		}
	}
	return strings.TrimSpace(best)
}

func parseOpenCodeJSONL(output string) string {
	best := ""
	for _, line := range strings.Split(output, "\n") {
		trimmed := strings.TrimSpace(line)
		if trimmed == "" {
			continue
		}

		var event map[string]interface{}
		if err := json.Unmarshal([]byte(trimmed), &event); err != nil {
			continue
		}

		if t, _ := event["type"].(string); t == "text" {
			if part, ok := event["part"].(map[string]interface{}); ok {
				if text, _ := part["text"].(string); strings.TrimSpace(text) != "" {
					best = text
				}
			}
		}

		if text, _ := event["text"].(string); strings.TrimSpace(text) != "" {
			best = text
		}
	}
	return strings.TrimSpace(best)
}

func extractCodexText(event map[string]interface{}) string {
	if t, _ := event["type"].(string); t == "item.completed" {
		if item, ok := event["item"].(map[string]interface{}); ok {
			if itemType, _ := item["type"].(string); itemType == "agent_message" {
				if text, _ := item["text"].(string); strings.TrimSpace(text) != "" {
					return text
				}
			}
		}
	}

	if text, _ := event["text"].(string); strings.TrimSpace(text) != "" {
		return text
	}
	if message, ok := event["message"].(map[string]interface{}); ok {
		if text, _ := message["text"].(string); strings.TrimSpace(text) != "" {
			return text
		}
	}
	return ""
}
