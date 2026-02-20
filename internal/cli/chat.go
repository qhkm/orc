package cli

import (
	"bufio"
	"context"
	"encoding/json"
	"errors"
	"flag"
	"fmt"
	"io"
	"os"
	"os/exec"
	"path/filepath"
	"regexp"
	"sort"
	"strconv"
	"strings"
	"sync"
	"sync/atomic"
	"time"

	"github.com/drnoraniza/orc/internal/flow"
	"github.com/drnoraniza/orc/internal/invoke"
	"github.com/drnoraniza/orc/internal/tui"
)

type chatRuntime struct {
	Flow            flow.Definition
	RepoPath        string
	Timeout         time.Duration
	ContinueSession bool
	ShowRawOutput   bool
	StreamOutput    bool
	VerboseOutput   bool
	StrictScope     bool
	UI              chatUI
	continueByAgent map[string]bool
	maxPrintedChars int
	lastStageID     string
	lastStageOutput string
}

type chatUI struct {
	theme tui.Theme
	live  bool
}

type chatScopeResult struct {
	Enabled bool   `json:"enabled"`
	Allowed bool   `json:"allowed"`
	Reply   string `json:"reply,omitempty"`
	Error   string `json:"error,omitempty"`
}

type chatStageResult struct {
	StageID    string   `json:"stage_id"`
	Intent     string   `json:"intent"`
	Provider   string   `json:"provider"`
	Model      string   `json:"model,omitempty"`
	DurationMs int64    `json:"duration_ms"`
	Command    []string `json:"command,omitempty"`
	Response   string   `json:"response"`
	RawOutput  string   `json:"raw_output,omitempty"`
}

type chatTurnResult struct {
	Request       string            `json:"request"`
	ScopeGuard    chatScopeResult   `json:"scope_guard"`
	Stages        []chatStageResult `json:"stages,omitempty"`
	FinalResponse string            `json:"final_response"`
}

type chatRunResult struct {
	RepoPath string         `json:"repo_path"`
	FlowPath string         `json:"flow_path"`
	Turn     chatTurnResult `json:"turn"`
}

func runChat(args []string, out io.Writer, errOut io.Writer) int {
	fs := flag.NewFlagSet("chat", flag.ContinueOnError)
	fs.SetOutput(errOut)

	var flowName string
	var flowPath string
	var repo string
	var once string
	var timeoutSeconds int
	var continueSession bool
	var showRawOutput bool
	var streamOutput bool
	var strictScope bool
	var liveProgress bool
	var verboseOutput bool
	var asJSON bool
	var maxOutputChars int

	fs.StringVar(&flowName, "flow", "default-flow", "flow name under .orc/flows")
	fs.StringVar(&flowPath, "flow-path", "", "explicit flow file path")
	fs.StringVar(&repo, "repo", "", "repository path (default current directory)")
	fs.StringVar(&once, "once", "", "run a single message and exit")
	fs.IntVar(&timeoutSeconds, "timeout", 180, "per-stage timeout in seconds")
	fs.BoolVar(&continueSession, "continue", true, "continue provider sessions between turns")
	fs.BoolVar(&showRawOutput, "show-raw", true, "print raw provider CLI output per stage")
	fs.BoolVar(&streamOutput, "stream", true, "stream provider output line-by-line while stage is running")
	fs.BoolVar(&strictScope, "strict-scope", true, "limit responses to coding/repository requests only")
	fs.BoolVar(&liveProgress, "live", true, "show live spinner/status while each stage is running")
	fs.BoolVar(&verboseOutput, "verbose", false, "print full stage output (disable compact previews)")
	fs.BoolVar(&asJSON, "json", false, "print single-turn result as JSON (requires --once)")
	fs.IntVar(&maxOutputChars, "max-output-chars", 8000, "max output chars per section (0 disables truncation)")

	if err := fs.Parse(args); err != nil {
		return 2
	}

	if asJSON && strings.TrimSpace(once) == "" {
		fmt.Fprintln(errOut, "--json requires --once in chat mode")
		return 2
	}

	repoPath, err := resolveRepoPath(repo)
	if err != nil {
		fmt.Fprintln(errOut, err)
		return 1
	}

	resolvedFlowPath, err := resolveChatFlowPath(repoPath, flowName, flowPath)
	if err != nil {
		fmt.Fprintln(errOut, err)
		return 1
	}

	definition, err := flow.Load(resolvedFlowPath)
	if err != nil {
		fmt.Fprintln(errOut, err)
		return 1
	}

	runtime := chatRuntime{
		Flow:            definition,
		RepoPath:        repoPath,
		Timeout:         time.Duration(timeoutSeconds) * time.Second,
		ContinueSession: continueSession,
		ShowRawOutput:   showRawOutput,
		StreamOutput:    streamOutput,
		VerboseOutput:   verboseOutput,
		StrictScope:     strictScope,
		UI:              newChatUI(out, liveProgress),
		continueByAgent: make(map[string]bool),
		maxPrintedChars: maxOutputChars,
	}

	if !asJSON {
		printChatHeader(out, resolvedFlowPath, runtime)
	}

	if strings.TrimSpace(once) != "" {
		runOut := out
		if asJSON {
			runOut = io.Discard
		}
		turn, err := runtime.runTurn(strings.TrimSpace(once), runOut, errOut)
		if err != nil {
			fmt.Fprintln(errOut, err)
			return 1
		}
		if asJSON {
			return writeJSON(out, errOut, chatRunResult{
				RepoPath: repoPath,
				FlowPath: resolvedFlowPath,
				Turn:     turn,
			})
		}
		return 0
	}

	scanner := bufio.NewScanner(os.Stdin)
	scanner.Buffer(make([]byte, 1024), 1024*1024)

	for {
		fmt.Fprint(out, runtime.UI.prompt("you"))
		if !scanner.Scan() {
			fmt.Fprintln(out)
			return 0
		}
		text := strings.TrimSpace(scanner.Text())
		if text == "" {
			continue
		}

		if strings.HasPrefix(text, "/") {
			handled, shouldExit := runtime.handleCommand(text, out, errOut)
			if shouldExit {
				return 0
			}
			if handled {
				continue
			}
		}

		turn, err := runtime.runTurn(text, out, errOut)
		if err != nil {
			fmt.Fprintf(errOut, "chat run failed: %v\n", err)
			continue
		}
		if runtime.maybePromptFollowUp(scanner, out, errOut, text, turn.FinalResponse) {
			return 0
		}
	}
}

func (r *chatRuntime) handleCommand(input string, out io.Writer, errOut io.Writer) (bool, bool) {
	switch strings.ToLower(strings.TrimSpace(input)) {
	case "/help":
		fmt.Fprintln(out, r.UI.dim("commands: /help, /flow, /reset, /view, /verbose, /exit"))
		fmt.Fprintln(out, r.UI.dim("after each run: choose [r] review, [x] cross-check, [i] refine, [v] view, [enter] next request"))
		fmt.Fprintln(out, r.UI.dim("streaming: use --stream=false to disable live stage output streaming"))
		return true, false
	case "/flow":
		printFlowStages(out, r.Flow)
		return true, false
	case "/reset":
		r.continueByAgent = make(map[string]bool)
		fmt.Fprintln(out, r.UI.green("session continuation state cleared"))
		return true, false
	case "/view":
		if strings.TrimSpace(r.lastStageOutput) == "" {
			fmt.Fprintln(out, r.UI.dim("no stage output available yet"))
			return true, false
		}
		fmt.Fprintln(out)
		fmt.Fprintln(out, r.UI.panel("full "+r.lastStageID, strings.Split(r.lastStageOutput, "\n")))
		return true, false
	case "/verbose":
		r.VerboseOutput = !r.VerboseOutput
		state := "off"
		if r.VerboseOutput {
			state = "on"
		}
		fmt.Fprintf(out, "%s %s\n", r.UI.dim("verbose output:"), state)
		return true, false
	case "/exit", "/quit":
		fmt.Fprintln(out, r.UI.dim("bye"))
		return true, true
	default:
		fmt.Fprintf(errOut, "unknown command %s (use /help)\n", input)
		return true, false
	}
}

func (r *chatRuntime) runTurn(userMessage string, out io.Writer, errOut io.Writer) (chatTurnResult, error) {
	turn := chatTurnResult{
		Request: userMessage,
		ScopeGuard: chatScopeResult{
			Enabled: r.StrictScope,
			Allowed: true,
		},
	}

	fmt.Fprintln(out)
	fmt.Fprintln(out, r.UI.panel("request", wrapForPanel(trimForDisplay(userMessage, r.maxPrintedChars), panelWrapWidth())))

	allowed, gateReply, err := r.scopeGuard(out, userMessage)
	if err != nil {
		turn.ScopeGuard.Error = err.Error()
		fmt.Fprintf(out, "%s scope guard degraded to local heuristic: %v\n", r.UI.dim("note:"), err)
	}
	if !allowed {
		turn.ScopeGuard.Allowed = false
		turn.ScopeGuard.Reply = gateReply
		turn.FinalResponse = gateReply
		fmt.Fprintln(out)
		fmt.Fprintln(out, r.UI.panel("scope guard", []string{r.UI.red("out_of_scope"), trimForDisplay(gateReply, r.maxPrintedChars)}))
		fmt.Fprintln(out, r.UI.panel("assistant", wrapForPanel(trimForDisplay(gateReply, r.maxPrintedChars), panelWrapWidth())))
		_ = errOut
		return turn, nil
	}

	original := userMessage
	current := userMessage
	for i, stage := range r.Flow.Stages {
		stageResult, err := r.runSingleStage(out, stage, i+1, len(r.Flow.Stages), original, current)
		if err != nil {
			return chatTurnResult{}, err
		}
		turn.Stages = append(turn.Stages, stageResult)
		current = stageResult.Response
	}

	turn.FinalResponse = current
	fmt.Fprintln(out)
	fmt.Fprintln(out, r.UI.panel("assistant", wrapForPanel(trimForDisplay(current, r.maxPrintedChars), panelWrapWidth())))
	_ = errOut
	return turn, nil
}

func (r *chatRuntime) scopeGuard(out io.Writer, userMessage string) (bool, string, error) {
	if !r.StrictScope {
		return true, "", nil
	}
	if isLikelyCodingOrRepoRequest(userMessage) {
		return true, "", nil
	}

	provider := normalizeChatProvider(strings.TrimSpace(r.Flow.MasterProvider))
	if provider == "" && len(r.Flow.Stages) > 0 {
		provider = normalizeChatProvider(r.Flow.Stages[0].Provider)
	}
	if provider == "" {
		return false, defaultScopeReply(), nil
	}

	model := stageModel(provider, "")
	prompt := buildScopeGuardPrompt(r.RepoPath, userMessage)
	result, err := invoke.Execute(context.Background(), invoke.Options{
		Provider:   provider,
		Model:      model,
		Message:    prompt,
		WorkingDir: r.RepoPath,
		Timeout:    minDuration(r.Timeout, 45*time.Second),
	})
	if err != nil {
		return false, defaultScopeReply(), err
	}

	decision, reply := parseScopeGuardDecision(result.Response)
	switch decision {
	case "in_scope":
		return true, "", nil
	case "out_of_scope":
		if strings.TrimSpace(reply) == "" {
			reply = defaultScopeReply()
		}
		return false, reply, nil
	default:
		return false, defaultScopeReply(), fmt.Errorf("unable to parse scope decision")
	}
}

func (r *chatRuntime) runSingleStage(out io.Writer, stage flow.Stage, stageNum, stageTotal int, originalRequest, previousOutput string) (chatStageResult, error) {
	provider := normalizeChatProvider(stage.Provider)
	if provider == "" {
		return chatStageResult{}, fmt.Errorf("stage %s has empty provider", stage.ID)
	}

	stagePrompt := r.buildStagePrompt(stage, originalRequest, previousOutput)
	model := stageModel(provider, stage.Profile)

	fmt.Fprintln(out)
	fmt.Fprintln(out, r.UI.panel(
		fmt.Sprintf("stage %d/%d %s", stageNum, stageTotal, r.UI.intentProviderBadge(stage.Intent, provider)),
		[]string{
			fmt.Sprintf("%s %s", r.UI.dim("id:"), stage.ID),
			fmt.Sprintf("%s %s", r.UI.dim("provider:"), provider),
			fmt.Sprintf("%s %s", r.UI.dim("intent:"), stage.Intent),
		},
	))
	if strings.TrimSpace(model) != "" {
		fmt.Fprintf(out, "%s %s\n", r.UI.dim("model:"), model)
	}

	result, err := r.executeStage(out, stage, stageNum, stageTotal, provider, model, stagePrompt)
	if err != nil {
		fmt.Fprintf(out, "%s %v\n", r.UI.red("status: failed"), err)
		return chatStageResult{}, fmt.Errorf("stage %s (%s) failed: %w", stage.ID, provider, err)
	}

	r.continueByAgent[provider] = true
	current := strings.TrimSpace(result.Response)
	if current == "" {
		current = strings.TrimSpace(result.RawOutput)
	}
	untruncatedCurrent := current
	current = trimForDisplay(current, r.maxPrintedChars)
	raw := ""
	if trimmedRaw := strings.TrimSpace(result.RawOutput); trimmedRaw != "" {
		raw = trimForDisplay(trimmedRaw, r.maxPrintedChars)
	}

	fmt.Fprintf(out, "%s %dms\n", r.UI.dim("duration:"), result.DurationMs)
	fmt.Fprintf(out, "%s %s\n", r.UI.dim("command:"), formatCommandForDisplay(result.Command, 220))
	responseSection, hidden := compactStageOutput(current, !r.VerboseOutput)
	responseTitle := "response"
	if hidden > 0 {
		responseTitle = fmt.Sprintf("response (%d lines hidden)", hidden)
	}
	fmt.Fprintln(out, r.UI.panel(responseTitle, wrapForPanel(responseSection, panelWrapWidth())))

	storedOutput := []string{
		"response:",
		current,
	}
	if r.ShowRawOutput && !r.StreamOutput && raw != "" && strings.TrimSpace(result.RawOutput) != strings.TrimSpace(untruncatedCurrent) {
		rawSection, rawHidden := compactStageOutput(raw, !r.VerboseOutput)
		rawTitle := "raw"
		if rawHidden > 0 {
			rawTitle = fmt.Sprintf("raw (%d lines hidden)", rawHidden)
		}
		fmt.Fprintln(out, r.UI.panel(rawTitle, wrapForPanel(rawSection, panelWrapWidth())))
		storedOutput = append(storedOutput, "", "raw:", raw)
	}

	r.lastStageID = stage.ID
	r.lastStageOutput = strings.Join(storedOutput, "\n")
	fmt.Fprintln(out, r.UI.green("status: done"))

	return chatStageResult{
		StageID:    stage.ID,
		Intent:     stage.Intent,
		Provider:   provider,
		Model:      model,
		DurationMs: result.DurationMs,
		Command:    result.Command,
		Response:   current,
		RawOutput:  raw,
	}, nil
}

func (r *chatRuntime) executeStage(out io.Writer, stage flow.Stage, stageNum, stageTotal int, provider, model, prompt string) (invoke.Result, error) {
	opts := invoke.Options{
		Provider:        provider,
		Model:           model,
		Message:         prompt,
		WorkingDir:      r.RepoPath,
		ContinueSession: r.ContinueSession && r.continueByAgent[provider],
		Timeout:         r.Timeout,
	}

	if r.StreamOutput {
		var printMu sync.Mutex
		progress := tui.NewProgressLine(out, r.UI.theme, r.UI.live)
		started := time.Now()
		doneCh := make(chan struct{})
		var sawAgentOutput atomic.Bool
		isJSONStream := provider == "claude" || provider == "codex" || provider == "opencode"
		stdoutPending := ""
		stderrPending := ""
		inlinePendingText := ""
		inlineMode := !r.VerboseOutput
		var inlineStarted atomic.Bool
		var inlineEnded atomic.Bool
		lastInlineChunk := ""
		lastInlineFlush := time.Time{}
		inlineFlushInterval := 220 * time.Millisecond
		var decodeMu sync.Mutex

		emit := func(prefix, msg string) {
			trimmed := strings.TrimSpace(msg)
			if trimmed == "" {
				return
			}
			printMu.Lock()
			defer printMu.Unlock()
			if r.UI.live {
				fmt.Fprint(out, "\r\x1b[2K\r")
			}
			if inlineMode && inlineStarted.Load() && !inlineEnded.Load() {
				fmt.Fprintln(out)
				inlineEnded.Store(true)
			}
			fmt.Fprintf(out, "%s %s\n", r.UI.dim(prefix), trimForDisplay(trimmed, r.maxPrintedChars))
		}

		emitInline := func(text string, forceEnd bool) {
			if text == "" && !forceEnd {
				return
			}
			printMu.Lock()
			defer printMu.Unlock()
			if r.UI.live && !inlineStarted.Load() {
				fmt.Fprint(out, "\r\x1b[2K\r")
			}
			if !inlineStarted.Load() {
				fmt.Fprintf(out, "%s ", r.UI.dim("stream>"))
				inlineStarted.Store(true)
			}
			if text != "" {
				fmt.Fprint(out, text)
			}
			if forceEnd && inlineStarted.Load() && !inlineEnded.Load() {
				fmt.Fprintln(out)
				inlineEnded.Store(true)
			}
		}

		emitDecoded := func(msg string, force bool) {
			decodeMu.Lock()
			defer decodeMu.Unlock()

			if strings.TrimSpace(msg) == "" && !force {
				return
			}
			if !inlineMode {
				sawAgentOutput.Store(true)
				emit("stream>", msg)
				return
			}

			normalized := strings.TrimSpace(msg)
			if normalized != "" {
				if len(normalized) > 32 && normalized == lastInlineChunk {
					if force {
						if strings.TrimSpace(inlinePendingText) != "" {
							sawAgentOutput.Store(true)
							emitInline(inlinePendingText, false)
							inlinePendingText = ""
						}
						emitInline("", true)
					}
					return
				}
				lastInlineChunk = normalized
			}
			// For non-Claude decoders, each decoded message is usually a complete line.
			// Add a newline delimiter so consecutive chunks don't glue together.
			if provider != "claude" && !strings.Contains(msg, "\n") {
				msg += "\n"
			}
			inlinePendingText += strings.ReplaceAll(msg, "\r", "\n")
			for {
				idx := strings.IndexByte(inlinePendingText, '\n')
				if idx < 0 {
					break
				}
				line := inlinePendingText[:idx]
				inlinePendingText = inlinePendingText[idx+1:]
				if strings.TrimSpace(line) == "" {
					continue
				}
				sawAgentOutput.Store(true)
				emitInline(line+"\n", false)
				lastInlineFlush = time.Now()
			}

			if !force {
				now := time.Now()
				canFlushByTime := lastInlineFlush.IsZero() || now.Sub(lastInlineFlush) >= inlineFlushInterval
				if canFlushByTime || len(inlinePendingText) >= 220 {
					minChunk := 28
					if !inlineStarted.Load() {
						minChunk = 12
					}
					cut := chooseInlineFlushCut(inlinePendingText, minChunk, 220)
					if cut > 0 {
						line := inlinePendingText[:cut]
						inlinePendingText = strings.TrimLeft(inlinePendingText[cut:], " ")
						if strings.TrimSpace(line) != "" {
							sawAgentOutput.Store(true)
							emitInline(line, false)
							lastInlineFlush = now
						}
					} else if canFlushByTime && len(strings.TrimSpace(inlinePendingText)) >= minChunk {
						// If provider tokens don't include spaces/punctuation yet, still flush periodically.
						sawAgentOutput.Store(true)
						emitInline(inlinePendingText, false)
						inlinePendingText = ""
						lastInlineFlush = now
					}
				}
			}
			if force && strings.TrimSpace(inlinePendingText) != "" {
				sawAgentOutput.Store(true)
				emitInline(inlinePendingText, false)
				lastInlineFlush = time.Now()
				inlinePendingText = ""
			}
			if force {
				emitInline("", true)
			}
		}

		handleStdoutChunk := func(chunk string, force bool) {
			if strings.TrimSpace(chunk) == "" && !force {
				return
			}
			if strings.TrimSpace(chunk) != "" {
				sawAgentOutput.Store(true)
			}

			if isJSONStream {
				stdoutPending += chunk
				for {
					idx := strings.IndexByte(stdoutPending, '\n')
					if idx < 0 {
						break
					}
					line := strings.TrimSpace(stdoutPending[:idx])
					stdoutPending = stdoutPending[idx+1:]
					if line == "" {
						continue
					}
					for _, msg := range decodeProviderStreamLine(provider, line, r.VerboseOutput) {
						emitDecoded(msg, false)
					}
				}
				if force && strings.TrimSpace(stdoutPending) != "" {
					for _, msg := range decodeProviderStreamLine(provider, strings.TrimSpace(stdoutPending), r.VerboseOutput) {
						emitDecoded(msg, true)
					}
					stdoutPending = ""
				}
				if force {
					emitDecoded("", true)
				}
				return
			}

			// Plain-text providers (claude/kimi): emit chunks immediately so first-stage output appears quickly.
			text := strings.ReplaceAll(chunk, "\r", "\n")
			parts := strings.Split(text, "\n")
			for _, part := range parts {
				if strings.TrimSpace(part) == "" {
					continue
				}
				emitDecoded(part, false)
			}
			if force {
				emitDecoded("", true)
			}
		}

		handleStderrChunk := func(chunk string, force bool) {
			stderrPending += chunk
			if strings.TrimSpace(chunk) != "" {
				sawAgentOutput.Store(true)
			}
			for {
				idx := strings.IndexByte(stderrPending, '\n')
				if idx < 0 {
					break
				}
				line := strings.TrimSpace(stderrPending[:idx])
				stderrPending = stderrPending[idx+1:]
				if line == "" {
					continue
				}
				if isJSONStream {
					decoded := decodeProviderStreamLine(provider, line, r.VerboseOutput)
					if len(decoded) > 0 {
						for _, msg := range decoded {
							emitDecoded(msg, false)
						}
						continue
					}
				}
				if !r.VerboseOutput && isIgnorableStreamStderr(provider, line) {
					continue
				}
				prefix := "stderr>"
				if !r.VerboseOutput {
					prefix = "warn>"
				}
				emit(prefix, line)
			}
			if force && strings.TrimSpace(stderrPending) != "" {
				line := strings.TrimSpace(stderrPending)
				stderrPending = ""
				if isJSONStream {
					decoded := decodeProviderStreamLine(provider, line, r.VerboseOutput)
					if len(decoded) > 0 {
						for _, msg := range decoded {
							emitDecoded(msg, true)
						}
						return
					}
				}
				if !(!r.VerboseOutput && isIgnorableStreamStderr(provider, line)) {
					prefix := "stderr>"
					if !r.VerboseOutput {
						prefix = "warn>"
					}
					emit(prefix, line)
				}
			}
		}

		go func() {
			ticker := time.NewTicker(120 * time.Millisecond)
			defer ticker.Stop()
			for {
				select {
				case <-doneCh:
					return
				case <-ticker.C:
					if inlineMode && inlineStarted.Load() {
						continue
					}
					printMu.Lock()
					progress.Tick(stageNum, stageTotal, stage.Intent, provider, time.Since(started))
					printMu.Unlock()
				}
			}
		}()
		go func() {
			timer := time.NewTimer(3 * time.Second)
			defer timer.Stop()
			select {
			case <-doneCh:
				return
			case <-timer.C:
				if sawAgentOutput.Load() {
					return
				}
				printMu.Lock()
				defer printMu.Unlock()
				if r.UI.live {
					fmt.Fprint(out, "\r\x1b[2K\r")
				}
				fmt.Fprintf(out, "%s waiting for first token from %s...\n", r.UI.dim("hint:"), provider)
			}
		}()

		onStdout := func(chunk string) {
			handleStdoutChunk(chunk, false)
		}
		onStderr := func(chunk string) {
			handleStderrChunk(chunk, false)
		}
		result, err := invoke.ExecuteStream(context.Background(), opts, onStdout, onStderr)
		handleStdoutChunk("", true)
		handleStderrChunk("", true)
		close(doneCh)
		printMu.Lock()
		progress.Stop(stageNum, stageTotal, stage.Intent, provider, time.Since(started), err)
		printMu.Unlock()
		return result, err
	}

	if !r.UI.live {
		return invoke.Execute(context.Background(), opts)
	}

	type stageResult struct {
		result invoke.Result
		err    error
	}

	done := make(chan stageResult, 1)
	started := time.Now()
	progress := tui.NewProgressLine(out, r.UI.theme, r.UI.live)
	go func() {
		result, err := invoke.Execute(context.Background(), opts)
		done <- stageResult{result: result, err: err}
	}()

	ticker := time.NewTicker(120 * time.Millisecond)
	defer ticker.Stop()

	for {
		select {
		case doneResult := <-done:
			progress.Stop(stageNum, stageTotal, stage.Intent, provider, time.Since(started), doneResult.err)
			return doneResult.result, doneResult.err
		case <-ticker.C:
			progress.Tick(stageNum, stageTotal, stage.Intent, provider, time.Since(started))
		}
	}
}

func decodeProviderStreamLine(provider, line string, verbose bool) []string {
	switch strings.TrimSpace(strings.ToLower(provider)) {
	case "claude":
		return decodeClaudeStreamLine(line, verbose)
	case "codex":
		return decodeCodexStreamLine(line, verbose)
	case "opencode":
		return decodeOpenCodeStreamLine(line, verbose)
	default:
		trimmed := strings.TrimSpace(line)
		if trimmed == "" {
			return nil
		}
		return []string{trimmed}
	}
}

func decodeClaudeStreamLine(line string, verbose bool) []string {
	trimmed := strings.TrimSpace(line)
	if trimmed == "" {
		return nil
	}

	var event map[string]interface{}
	if err := json.Unmarshal([]byte(trimmed), &event); err != nil {
		if !strings.HasPrefix(trimmed, "{") {
			return splitNonEmptyLinesPreserveWhitespace(line)
		}
		if verbose {
			return []string{trimmed}
		}
		return nil
	}

	return decodeClaudeEvent(event, verbose)
}

func decodeClaudeEvent(event map[string]interface{}, verbose bool) []string {
	if nested, ok := event["event"].(map[string]interface{}); ok {
		if lines := decodeClaudeEvent(nested, verbose); len(lines) > 0 {
			return lines
		}
	}

	eventType, _ := event["type"].(string)
	switch eventType {
	case "content_block_delta", "message_delta":
		if lines := claudeTextFromDelta(event["delta"]); len(lines) > 0 {
			return lines
		}
		return nil
	case "content_block_start":
		if block, ok := event["content_block"].(map[string]interface{}); ok {
			if text, _ := block["text"].(string); strings.TrimSpace(text) != "" {
				return splitNonEmptyLinesPreserveWhitespace(text)
			}
		}
		return nil
	case "assistant":
		if verbose {
			if message, ok := event["message"].(map[string]interface{}); ok {
				if lines := claudeTextFromMessage(message); len(lines) > 0 {
					return lines
				}
			}
		}
		return nil
	case "result":
		if verbose {
			if result, _ := event["result"].(string); strings.TrimSpace(result) != "" {
				return splitNonEmptyLines(result)
			}
		}
		return nil
	}

	if lines := claudeTextFromDelta(event["delta"]); len(lines) > 0 {
		return lines
	}
	if message, ok := event["message"].(map[string]interface{}); ok {
		if lines := claudeTextFromMessage(message); len(lines) > 0 {
			return lines
		}
	}
	if lines := claudeTextFromContent(event["content"]); len(lines) > 0 {
		return lines
	}
	if text, _ := event["text"].(string); strings.TrimSpace(text) != "" {
		return splitNonEmptyLines(text)
	}

	if verbose && strings.TrimSpace(eventType) != "" {
		return []string{fmt.Sprintf("[claude:%s]", eventType)}
	}
	return nil
}

func claudeTextFromMessage(message map[string]interface{}) []string {
	if lines := claudeTextFromContent(message["content"]); len(lines) > 0 {
		return lines
	}
	if text, _ := message["text"].(string); strings.TrimSpace(text) != "" {
		return splitNonEmptyLines(text)
	}
	return nil
}

func claudeTextFromContent(value interface{}) []string {
	blocks, ok := value.([]interface{})
	if !ok {
		return nil
	}
	texts := make([]string, 0, len(blocks))
	for _, block := range blocks {
		blockMap, ok := block.(map[string]interface{})
		if !ok {
			continue
		}
		if blockType, _ := blockMap["type"].(string); blockType != "" && blockType != "text" {
			continue
		}
		if text, _ := blockMap["text"].(string); strings.TrimSpace(text) != "" {
			texts = append(texts, text)
		}
	}
	if len(texts) == 0 {
		return nil
	}
	return splitNonEmptyLines(strings.Join(texts, "\n"))
}

func claudeTextFromDelta(value interface{}) []string {
	delta, ok := value.(map[string]interface{})
	if !ok {
		return nil
	}
	if text, _ := delta["text"].(string); strings.TrimSpace(text) != "" {
		return splitNonEmptyLinesPreserveWhitespace(text)
	}
	return nil
}

func decodeCodexStreamLine(line string, verbose bool) []string {
	var event map[string]interface{}
	if err := json.Unmarshal([]byte(line), &event); err != nil {
		if verbose {
			trimmed := strings.TrimSpace(line)
			if trimmed != "" {
				return []string{trimmed}
			}
		}
		return nil
	}

	if t, _ := event["type"].(string); t == "item.completed" {
		if item, ok := event["item"].(map[string]interface{}); ok {
			itemType, _ := item["type"].(string)
			switch itemType {
			case "agent_message":
				if text, _ := item["text"].(string); strings.TrimSpace(text) != "" {
					return splitNonEmptyLines(text)
				}
			case "reasoning":
				if verbose {
					if text, _ := item["text"].(string); strings.TrimSpace(text) != "" {
						return []string{"[reasoning] " + strings.TrimSpace(text)}
					}
				}
			case "command_execution":
				if verbose {
					command, _ := item["command"].(string)
					status, _ := item["status"].(string)
					command = strings.TrimSpace(command)
					status = strings.TrimSpace(status)
					if command != "" || status != "" {
						return []string{fmt.Sprintf("[tool:%s] %s", nonEmptyOr(status, "unknown"), command)}
					}
				}
			}
		}
	}

	if verbose {
		if text, _ := event["text"].(string); strings.TrimSpace(text) != "" {
			return splitNonEmptyLines(text)
		}
	}
	return nil
}

func decodeOpenCodeStreamLine(line string, verbose bool) []string {
	var event map[string]interface{}
	if err := json.Unmarshal([]byte(line), &event); err != nil {
		if verbose {
			trimmed := strings.TrimSpace(line)
			if trimmed != "" {
				return []string{trimmed}
			}
		}
		return nil
	}

	if t, _ := event["type"].(string); t == "text" {
		if part, ok := event["part"].(map[string]interface{}); ok {
			if text, _ := part["text"].(string); strings.TrimSpace(text) != "" {
				return splitNonEmptyLines(text)
			}
		}
	}

	if verbose {
		if text, _ := event["text"].(string); strings.TrimSpace(text) != "" {
			return splitNonEmptyLines(text)
		}
	}
	return nil
}

func splitNonEmptyLines(value string) []string {
	lines := strings.Split(strings.TrimSpace(value), "\n")
	out := make([]string, 0, len(lines))
	for _, line := range lines {
		trimmed := strings.TrimSpace(line)
		if trimmed == "" {
			continue
		}
		out = append(out, trimmed)
	}
	return out
}

func splitNonEmptyLinesPreserveWhitespace(value string) []string {
	lines := strings.Split(strings.ReplaceAll(value, "\r\n", "\n"), "\n")
	out := make([]string, 0, len(lines))
	for _, line := range lines {
		if strings.TrimSpace(line) == "" {
			continue
		}
		out = append(out, line)
	}
	return out
}

func isIgnorableStreamStderr(provider, line string) bool {
	lower := strings.ToLower(strings.TrimSpace(line))
	if lower == "" {
		return true
	}

	// Known noisy codex background warnings that do not affect stage result.
	if strings.Contains(lower, "rollout::list: state db missing rollout path") {
		return true
	}
	if strings.Contains(lower, "failed to record rollout items: failed to queue rollout items: channel closed") {
		return true
	}

	// OpenCode connectivity noise that is not task output.
	if strings.Contains(lower, "service=models.dev") && strings.Contains(lower, "unable to connect") {
		return true
	}

	_ = provider
	return false
}

func (r *chatRuntime) maybePromptFollowUp(scanner *bufio.Scanner, out io.Writer, errOut io.Writer, originalRequest, latestOutput string) bool {
	current := latestOutput

	for {
		fmt.Fprintln(out)
		fmt.Fprintln(out, r.UI.dim("next action: [enter] new request | [r] review | [x] cross-check | [i] refine implement | [v] view full output | [q] quit"))
		fmt.Fprint(out, r.UI.prompt("orc"))
		if !scanner.Scan() {
			fmt.Fprintln(out)
			return true
		}
		choice := strings.ToLower(strings.TrimSpace(scanner.Text()))

		switch choice {
		case "", "n", "new", "next":
			return false
		case "q", "quit", "/exit":
			fmt.Fprintln(out, r.UI.dim("bye"))
			return true
		case "r", "review":
			provider, shouldQuit := r.promptProvider(scanner, out, "review provider", r.defaultProviderForIntent("review"))
			if shouldQuit {
				return true
			}
			if provider == "" {
				continue
			}
			stage := flow.Stage{
				ID:       fmt.Sprintf("review_followup_%d", time.Now().Unix()),
				Intent:   "review",
				Provider: provider,
			}
			stageResult, err := r.runSingleStage(out, stage, 1, 1, originalRequest, current)
			if err != nil {
				fmt.Fprintf(errOut, "follow-up review failed: %v\n", err)
				continue
			}
			current = stageResult.Response
		case "x", "cross", "cross-check":
			provider, shouldQuit := r.promptProvider(scanner, out, "cross-check provider", r.defaultProviderForIntent("cross_check"))
			if shouldQuit {
				return true
			}
			if provider == "" {
				continue
			}
			stage := flow.Stage{
				ID:       fmt.Sprintf("cross_check_followup_%d", time.Now().Unix()),
				Intent:   "cross_check",
				Provider: provider,
			}
			stageResult, err := r.runSingleStage(out, stage, 1, 1, originalRequest, current)
			if err != nil {
				fmt.Fprintf(errOut, "follow-up cross-check failed: %v\n", err)
				continue
			}
			current = stageResult.Response
		case "i", "implement", "refine":
			provider, shouldQuit := r.promptProvider(scanner, out, "implement provider", r.defaultProviderForIntent("implement"))
			if shouldQuit {
				return true
			}
			if provider == "" {
				continue
			}
			stage := flow.Stage{
				ID:       fmt.Sprintf("implement_followup_%d", time.Now().Unix()),
				Intent:   "implement",
				Provider: provider,
			}
			stageResult, err := r.runSingleStage(out, stage, 1, 1, originalRequest, current)
			if err != nil {
				fmt.Fprintf(errOut, "follow-up implement failed: %v\n", err)
				continue
			}
			current = stageResult.Response
		case "v", "view":
			if strings.TrimSpace(r.lastStageOutput) == "" {
				fmt.Fprintln(out, r.UI.dim("no stage output available yet"))
				continue
			}
			fmt.Fprintln(out, r.UI.panel("full "+r.lastStageID, strings.Split(r.lastStageOutput, "\n")))
		default:
			fmt.Fprintln(out, r.UI.dim("invalid option; choose enter/r/x/i/v/q"))
		}
	}
}

func (r *chatRuntime) promptProvider(scanner *bufio.Scanner, out io.Writer, label, defaultProvider string) (string, bool) {
	options := "claude/codex/kimi/opencode"
	for {
		fmt.Fprintf(out, "%s %s [%s] (default %s, q to quit): ", r.UI.dim(label+":"), options, options, defaultProvider)
		if !scanner.Scan() {
			fmt.Fprintln(out)
			return "", true
		}
		value := strings.TrimSpace(strings.ToLower(scanner.Text()))
		if value == "q" || value == "quit" {
			fmt.Fprintln(out, r.UI.dim("bye"))
			return "", true
		}
		if value == "" {
			return defaultProvider, false
		}
		normalized := normalizeChatProvider(value)
		if isSupportedFollowUpProvider(normalized) {
			return normalized, false
		}
		fmt.Fprintln(out, r.UI.dim("unsupported provider; use claude/codex/kimi/opencode"))
	}
}

func (r *chatRuntime) defaultProviderForIntent(intent string) string {
	normalizedIntent := strings.ToLower(strings.TrimSpace(intent))
	for _, stage := range r.Flow.Stages {
		if strings.EqualFold(strings.TrimSpace(stage.Intent), normalizedIntent) {
			provider := normalizeChatProvider(stage.Provider)
			if isSupportedFollowUpProvider(provider) {
				return provider
			}
		}
	}
	switch normalizedIntent {
	case "cross_check":
		return "kimi"
	case "review":
		return "codex"
	case "implement":
		return "codex"
	default:
		return "codex"
	}
}

func isSupportedFollowUpProvider(provider string) bool {
	switch strings.TrimSpace(strings.ToLower(provider)) {
	case "claude", "codex", "kimi", "opencode":
		return true
	default:
		return false
	}
}

func compactStageOutput(content string, compact bool) (string, int) {
	if !compact {
		return content, 0
	}
	lines := strings.Split(content, "\n")
	if len(lines) <= 3 {
		return content, 0
	}
	hidden := len(lines) - 3
	preview := append(lines[:3], fmt.Sprintf("...%d more lines (use --verbose or /view)", hidden))
	return strings.Join(preview, "\n"), hidden
}

func (r *chatRuntime) buildStagePrompt(stage flow.Stage, originalRequest, previousOutput string) string {
	intent := strings.ToLower(strings.TrimSpace(stage.Intent))
	builder := &strings.Builder{}

	builder.WriteString("You are part of a multi-agent coding workflow.\n")
	builder.WriteString("Scope policy: answer only coding tasks related to the current repository. If out-of-scope, respond with OUT_OF_SCOPE and a one-line redirect.\n")
	builder.WriteString(fmt.Sprintf("Repository path: %s\n", r.RepoPath))
	builder.WriteString(fmt.Sprintf("Current stage: %s (%s)\n\n", stage.ID, stage.Intent))
	builder.WriteString("User request:\n")
	builder.WriteString(originalRequest)
	builder.WriteString("\n\n")

	if previousOutput != originalRequest {
		builder.WriteString("Output from previous stage:\n")
		builder.WriteString(previousOutput)
		builder.WriteString("\n\n")
	}

	switch intent {
	case "brainstorm":
		builder.WriteString("Task: Produce a concise technical plan (3-7 bullets) for implementation.\n")
	case "implement":
		builder.WriteString("Task: Implement the requested changes directly in the repository. Then summarize changed files and key decisions.\n")
	case "cross_check":
		builder.WriteString("Task: Cross-check implementation for obvious bugs, regressions, and missing tests. Return findings clearly.\n")
	case "review":
		builder.WriteString("Task: Review the implementation and provide PASS/FAIL plus issues and recommendations.\n")
	default:
		builder.WriteString("Task: Complete this stage and provide output suitable for the next stage.\n")
	}

	if intent == "review" || intent == "cross_check" {
		if diff := readGitDiff(r.RepoPath, 12000); strings.TrimSpace(diff) != "" {
			builder.WriteString("\nCurrent git diff:\n")
			builder.WriteString(diff)
			builder.WriteString("\n")
		}
	}

	builder.WriteString("\nRespond with concise, high-signal output only.")
	return builder.String()
}

func readGitDiff(repoPath string, maxChars int) string {
	cmd := exec.Command("git", "-C", repoPath, "diff")
	output, err := cmd.Output()
	if err != nil {
		return ""
	}
	text := strings.TrimSpace(string(output))
	if maxChars > 0 && len(text) > maxChars {
		return text[:maxChars] + "\n[truncated]"
	}
	return text
}

func trimForDisplay(value string, maxChars int) string {
	trimmed := strings.TrimSpace(value)
	if trimmed == "" {
		return "(empty response)"
	}
	if maxChars > 0 && len(trimmed) > maxChars {
		return trimmed[:maxChars] + "\n...[truncated]"
	}
	return trimmed
}

func panelWrapWidth() int {
	if raw := strings.TrimSpace(os.Getenv("COLUMNS")); raw != "" {
		if width, err := strconv.Atoi(raw); err == nil && width >= 60 {
			return width - 8
		}
	}
	return 96
}

func wrapForPanel(content string, width int) []string {
	lines := strings.Split(strings.ReplaceAll(content, "\r\n", "\n"), "\n")
	if width <= 0 {
		return lines
	}
	out := make([]string, 0, len(lines))
	for _, line := range lines {
		out = append(out, wrapLine(line, width)...)
	}
	return out
}

func wrapLine(line string, width int) []string {
	trimmed := strings.TrimSpace(line)
	if trimmed == "" || width <= 0 || len([]rune(line)) <= width {
		return []string{line}
	}

	words := strings.Fields(line)
	if len(words) == 0 {
		return []string{line}
	}

	out := make([]string, 0, (len([]rune(line))/width)+1)
	current := words[0]
	for _, word := range words[1:] {
		candidate := current + " " + word
		if len([]rune(candidate)) <= width {
			current = candidate
			continue
		}
		out = append(out, current)
		current = word
	}
	out = append(out, current)
	return out
}

func chooseInlineFlushCut(value string, minChunk, maxScan int) int {
	if minChunk < 1 {
		minChunk = 1
	}
	if maxScan <= 0 || len(value) < maxScan {
		maxScan = len(value)
	}
	scan := value[:maxScan]

	best := -1
	for _, marker := range []string{". ", "! ", "? ", "; ", ": ", ", "} {
		if idx := strings.LastIndex(scan, marker); idx >= minChunk {
			cut := idx + len(marker)
			if cut > best {
				best = cut
			}
		}
	}
	if best > 0 {
		return best
	}

	if idx := strings.LastIndex(scan, " "); idx >= minChunk {
		return idx + 1
	}
	if len(scan) >= 120 {
		return len(scan)
	}
	return -1
}

func formatCommandForDisplay(command []string, maxChars int) string {
	if len(command) == 0 {
		return "(none)"
	}
	display := make([]string, 0, len(command))
	for i, arg := range command {
		masked := arg
		isLast := i == len(command)-1
		if strings.Contains(masked, "\n") {
			if isLast {
				masked = fmt.Sprintf("<prompt:%d chars>", len(arg))
			} else {
				masked = fmt.Sprintf("<multiline:%d chars>", len(arg))
			}
		} else if isLast && len(masked) > 120 {
			masked = fmt.Sprintf("<prompt:%d chars>", len(arg))
		} else if len(masked) > 80 {
			masked = masked[:77] + "..."
		}
		display = append(display, masked)
	}
	result := strings.Join(display, " ")
	if maxChars > 0 && len(result) > maxChars {
		return result[:maxChars] + " ..."
	}
	return result
}

func stageModel(provider, profile string) string {
	normalizedProfile := strings.ToLower(strings.TrimSpace(profile))
	switch provider {
	case "claude":
		switch normalizedProfile {
		case "opus", "claude-opus-4-6":
			return "claude-opus-4-6"
		case "sonnet", "claude-sonnet-4-5":
			return "claude-sonnet-4-5"
		default:
			return ""
		}
	case "codex":
		return strings.TrimSpace(profile)
	default:
		return strings.TrimSpace(profile)
	}
}

func normalizeChatProvider(provider string) string {
	normalized := strings.ToLower(strings.TrimSpace(provider))
	switch normalized {
	case "anthropic":
		return "claude"
	case "openai":
		return "codex"
	default:
		return normalized
	}
}

func resolveChatFlowPath(repoPath, flowName, explicitFlowPath string) (string, error) {
	if strings.TrimSpace(explicitFlowPath) != "" {
		return resolveFilePath(explicitFlowPath)
	}

	normalized := flow.NormalizeName(flowName)
	if normalized == "" {
		return "", fmt.Errorf("flow name is required")
	}
	candidate := filepath.Join(repoPath, ".orc", "flows", normalized+".json")
	if _, err := os.Stat(candidate); err == nil {
		return candidate, nil
	} else if !errors.Is(err, os.ErrNotExist) {
		return "", fmt.Errorf("check flow file: %w", err)
	}

	fallback, err := firstFlowFile(repoPath)
	if err != nil {
		return "", err
	}
	if fallback != "" {
		return fallback, nil
	}
	return "", fmt.Errorf("flow %q not found and no flow files exist under %s", normalized, filepath.Join(repoPath, ".orc", "flows"))
}

func firstFlowFile(repoPath string) (string, error) {
	flowsDir := filepath.Join(repoPath, ".orc", "flows")
	entries, err := os.ReadDir(flowsDir)
	if err != nil {
		if errors.Is(err, os.ErrNotExist) {
			return "", nil
		}
		return "", fmt.Errorf("list flow directory: %w", err)
	}

	candidates := make([]string, 0, len(entries))
	for _, entry := range entries {
		if entry.IsDir() {
			continue
		}
		name := entry.Name()
		if filepath.Ext(name) != ".json" {
			continue
		}
		candidates = append(candidates, filepath.Join(flowsDir, name))
	}

	if len(candidates) == 0 {
		return "", nil
	}
	sort.Strings(candidates)
	return candidates[0], nil
}

func printChatHeader(out io.Writer, flowPath string, runtime chatRuntime) {
	lines := []string{
		fmt.Sprintf("%s %s", runtime.UI.dim("repo:"), runtime.RepoPath),
		fmt.Sprintf("%s %s", runtime.UI.dim("flow:"), flowPath),
		fmt.Sprintf("%s %d", runtime.UI.dim("stages:"), len(runtime.Flow.Stages)),
	}
	fmt.Fprintln(out, runtime.UI.panel(runtime.UI.cyanBold("orc terminal"), lines))
	printFlowStages(out, runtime.Flow)
	fmt.Fprintln(out, runtime.UI.dim("type /help for commands"))
	fmt.Fprintln(out)
}

func printFlowStages(out io.Writer, def flow.Definition) {
	fmt.Fprintln(out, "Stage pipeline:")
	theme := tui.NewTheme(out)
	for i, stage := range def.Stages {
		line := fmt.Sprintf("  %d) %s", i+1, tui.IntentProviderBadge(theme, stage.Intent, normalizeChatProvider(stage.Provider)))
		if strings.TrimSpace(stage.Profile) != "" {
			line += fmt.Sprintf(" profile=%s", stage.Profile)
		}
		fmt.Fprintln(out, line)
	}
}

func newChatUI(out io.Writer, liveEnabled bool) chatUI {
	theme := tui.NewTheme(out)
	live := liveEnabled && tui.IsTerminalWriter(out)
	if strings.EqualFold(strings.TrimSpace(os.Getenv("ORC_NO_LIVE")), "1") {
		live = false
	}
	if !theme.Color {
		live = false
	}
	return chatUI{theme: theme, live: live}
}

func (u chatUI) rule(symbol string) string {
	return tui.Rule(symbol, 72)
}

func (u chatUI) label(value string) string {
	return u.theme.Bold(u.theme.Cyan(value))
}

func (u chatUI) dim(value string) string {
	return u.theme.Dim(value)
}

func (u chatUI) cyan(value string) string {
	return u.theme.Cyan(value)
}

func (u chatUI) cyanBold(value string) string {
	return u.theme.Bold(u.theme.Cyan(value))
}

func (u chatUI) green(value string) string {
	return u.theme.Green(value)
}

func (u chatUI) red(value string) string {
	return u.theme.Red(value)
}

func (u chatUI) prompt(actor string) string {
	return u.theme.Prompt(actor)
}

func (u chatUI) panel(title string, lines []string) string {
	return tui.Box(title, lines)
}

func (u chatUI) intentProviderBadge(intent, provider string) string {
	return tui.IntentProviderBadge(u.theme, intent, provider)
}

type scopeGuardPayload struct {
	Decision string `json:"decision"`
	Reply    string `json:"reply"`
}

var codingHintPattern = regexp.MustCompile(`(?i)(\b(code|coding|implement|fix|bug|issue|test|refactor|review|build|compile|run|repo|repository|file|function|class|module|api|endpoint|cli|workflow|yaml|json|toml|sql|rust|golang|go|python|javascript|typescript|node|cargo|npm|pnpm|git|commit|branch|diff|merge|pr)\b|[A-Za-z0-9_\-\/]+\.(go|rs|ts|tsx|js|jsx|py|java|kt|swift|json|yaml|yml|toml|md))`)

func isLikelyCodingOrRepoRequest(message string) bool {
	trimmed := strings.TrimSpace(strings.ToLower(message))
	if trimmed == "" {
		return false
	}
	switch trimmed {
	case "hi", "hello", "hey", "yo", "sup", "how are you", "good morning", "good afternoon", "good evening":
		return false
	}
	return codingHintPattern.MatchString(message)
}

func buildScopeGuardPrompt(repoPath, userMessage string) string {
	payload := map[string]string{
		"decision": "in_scope|out_of_scope",
		"reply":    "short response for user",
	}
	template := `You are the scope gatekeeper for a coding CLI.
Repository path: %s

Decide if the request is in scope.
In scope: coding tasks, repository tasks, debugging, tests, implementation, review.
Out of scope: greetings/chit-chat/general Q&A unrelated to coding or the current repository.

User request:
%s

Return only strict JSON (no markdown, no extra keys):
%s`
	return fmt.Sprintf(template, repoPath, userMessage, mustJSON(payload))
}

func parseScopeGuardDecision(response string) (decision string, reply string) {
	trimmed := strings.TrimSpace(response)
	if trimmed == "" {
		return "", ""
	}

	var payload scopeGuardPayload
	if err := json.Unmarshal([]byte(trimmed), &payload); err == nil {
		return strings.ToLower(strings.TrimSpace(payload.Decision)), strings.TrimSpace(payload.Reply)
	}

	start := strings.Index(trimmed, "{")
	end := strings.LastIndex(trimmed, "}")
	if start >= 0 && end > start {
		if err := json.Unmarshal([]byte(trimmed[start:end+1]), &payload); err == nil {
			return strings.ToLower(strings.TrimSpace(payload.Decision)), strings.TrimSpace(payload.Reply)
		}
	}

	lower := strings.ToLower(trimmed)
	if strings.Contains(lower, "out_of_scope") || strings.Contains(lower, "out of scope") {
		return "out_of_scope", trimmed
	}
	if strings.Contains(lower, "in_scope") || strings.Contains(lower, "in scope") {
		return "in_scope", trimmed
	}
	return "", ""
}

func defaultScopeReply() string {
	return "I can help only with coding and tasks related to this repository. Ask me to implement, debug, review, or explain code/files in the current repo."
}

func mustJSON(value interface{}) string {
	payload, _ := json.Marshal(value)
	return string(payload)
}

func minDuration(a, b time.Duration) time.Duration {
	if a <= 0 {
		return b
	}
	if b <= 0 {
		return a
	}
	if a < b {
		return a
	}
	return b
}
