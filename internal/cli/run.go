package cli

import (
	"context"
	"encoding/json"
	"errors"
	"flag"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"strings"
	"time"

	"github.com/drnoraniza/orc/internal/config"
	"github.com/drnoraniza/orc/internal/doctor"
	"github.com/drnoraniza/orc/internal/flow"
	"github.com/drnoraniza/orc/internal/handoff"
	"github.com/drnoraniza/orc/internal/invoke"
	"github.com/drnoraniza/orc/internal/providers"
	"github.com/drnoraniza/orc/internal/tui"
)

type initResult struct {
	ConfigPath     string            `json:"config_path"`
	MasterProvider string            `json:"master_provider"`
	Providers      []config.Provider `json:"providers"`
}

type flowCreateResult struct {
	FlowPath string          `json:"flow_path"`
	Flow     flow.Definition `json:"flow"`
}

func Run(args []string, out io.Writer, errOut io.Writer) int {
	if len(args) == 0 {
		return runChat([]string{}, out, errOut)
	}
	if args[0] == "help" || args[0] == "-h" || args[0] == "--help" {
		printRootHelp(out)
		return 0
	}
	if strings.HasPrefix(args[0], "-") {
		return runChat(args, out, errOut)
	}

	switch args[0] {
	case "init":
		return runInit(args[1:], out, errOut)
	case "providers":
		return runProviders(args[1:], out, errOut)
	case "flow":
		return runFlow(args[1:], out, errOut)
	case "chat":
		return runChat(args[1:], out, errOut)
	case "invoke":
		return runInvoke(args[1:], out, errOut)
	case "handoff":
		return runHandoff(args[1:], out, errOut)
	case "continue":
		return runContinue(args[1:], out, errOut)
	case "status":
		return runStatus(args[1:], out, errOut)
	default:
		fmt.Fprintf(errOut, "unknown command %q\n\n", args[0])
		printRootHelp(errOut)
		return 1
	}
}

func runInit(args []string, out io.Writer, errOut io.Writer) int {
	fs := flag.NewFlagSet("init", flag.ContinueOnError)
	fs.SetOutput(errOut)

	var master string
	var configPath string
	var asJSON bool
	var interactive bool
	fs.StringVar(&master, "master", "", "master provider name")
	fs.StringVar(&configPath, "config", "", "config path (default ~/.orc/config.json)")
	fs.BoolVar(&asJSON, "json", false, "print result as JSON")
	fs.BoolVar(&interactive, "interactive", false, "run interactive onboarding wizard")

	if err := fs.Parse(args); err != nil {
		return 2
	}
	if asJSON && interactive {
		fmt.Fprintln(errOut, "--json and --interactive cannot be used together")
		return 2
	}

	reports, err := doctor.Run(context.Background(), providers.DefaultRegistry(), doctor.Options{})
	if err != nil {
		fmt.Fprintln(errOut, err)
		return 1
	}

	registered := make([]config.Provider, 0, len(reports))
	for _, report := range reports {
		if !report.Installed {
			continue
		}
		registered = append(registered, config.Provider{
			Name:                   normalizeProviderName(report.Name),
			Binary:                 report.Binary,
			Installed:              report.Installed,
			Version:                report.Version,
			AuthStatus:             string(report.AuthStatus),
			SupportsNonInteractive: report.SupportsNonInteractive,
			SupportsJSON:           report.SupportsJSON,
			Notes:                  report.Notes,
		})
	}

	if len(registered) == 0 {
		fmt.Fprintln(errOut, "no supported provider CLIs detected on PATH")
		return 1
	}

	resolvedConfigPath, err := config.ResolvePath(configPath)
	if err != nil {
		fmt.Fprintln(errOut, err)
		return 1
	}

	selectedProviders := registered
	selectedMaster := ""
	if interactive {
		wizard, err := runInitWizard(os.Stdin, out, registered, master, resolvedConfigPath)
		if err != nil {
			if errors.Is(err, errInitCanceled) {
				fmt.Fprintln(errOut, "init canceled")
				return 1
			}
			fmt.Fprintln(errOut, err)
			return 1
		}
		selectedProviders = wizard.Providers
		selectedMaster = wizard.MasterProvider
		resolvedConfigPath, err = config.ResolvePath(wizard.ConfigPath)
		if err != nil {
			fmt.Fprintln(errOut, err)
			return 1
		}
	} else {
		selectedMaster, err = chooseMasterProvider(master, registered)
		if err != nil {
			fmt.Fprintln(errOut, err)
			return 1
		}
	}

	createdAt := time.Now().UTC()
	existing, loadErr := config.Load(resolvedConfigPath)
	if loadErr == nil && !existing.CreatedAt.IsZero() {
		createdAt = existing.CreatedAt
	} else if loadErr != nil && !errors.Is(loadErr, os.ErrNotExist) {
		fmt.Fprintln(errOut, loadErr)
		return 1
	}

	cfg := config.File{
		MasterProvider: selectedMaster,
		Providers:      selectedProviders,
		CreatedAt:      createdAt,
		UpdatedAt:      time.Now().UTC(),
	}
	if err := config.Save(resolvedConfigPath, cfg); err != nil {
		fmt.Fprintln(errOut, err)
		return 1
	}

	result := initResult{
		ConfigPath:     resolvedConfigPath,
		MasterProvider: selectedMaster,
		Providers:      selectedProviders,
	}
	if asJSON {
		return writeJSON(out, errOut, result)
	}

	printInitSummary(out, result)
	return 0
}

func runProviders(args []string, out io.Writer, errOut io.Writer) int {
	if len(args) == 0 || args[0] == "help" || args[0] == "-h" || args[0] == "--help" {
		printProvidersHelp(out)
		return 0
	}

	switch args[0] {
	case "doctor":
		fs := flag.NewFlagSet("doctor", flag.ContinueOnError)
		fs.SetOutput(errOut)

		var provider string
		var asJSON bool
		fs.StringVar(&provider, "provider", "", "check a single provider")
		fs.BoolVar(&asJSON, "json", false, "print report as JSON")

		if err := fs.Parse(args[1:]); err != nil {
			return 2
		}

		reports, err := doctor.Run(context.Background(), providers.DefaultRegistry(), doctor.Options{Provider: provider})
		if err != nil {
			fmt.Fprintln(errOut, err)
			return 1
		}

		if asJSON {
			enc := json.NewEncoder(out)
			enc.SetIndent("", "  ")
			if err := enc.Encode(reports); err != nil {
				fmt.Fprintln(errOut, err)
				return 1
			}
			return 0
		}

		printDoctorTable(out, reports)
		return 0
	default:
		fmt.Fprintf(errOut, "unknown providers command %q\n\n", args[0])
		printProvidersHelp(errOut)
		return 1
	}
}

func runFlow(args []string, out io.Writer, errOut io.Writer) int {
	if len(args) == 0 || args[0] == "help" || args[0] == "-h" || args[0] == "--help" {
		printFlowHelp(out)
		return 0
	}

	switch args[0] {
	case "create":
		return runFlowCreate(args[1:], out, errOut)
	default:
		fmt.Fprintf(errOut, "unknown flow command %q\n\n", args[0])
		printFlowHelp(errOut)
		return 1
	}
}

func runFlowCreate(args []string, out io.Writer, errOut io.Writer) int {
	fs := flag.NewFlagSet("flow create", flag.ContinueOnError)
	fs.SetOutput(errOut)

	var name string
	var repo string
	var outputPath string
	var configPath string
	var master string
	var prompt string
	var asJSON bool

	fs.StringVar(&name, "name", "default-flow", "flow name")
	fs.StringVar(&repo, "repo", "", "repository path (default current directory)")
	fs.StringVar(&outputPath, "output", "", "flow output file path")
	fs.StringVar(&configPath, "config", "", "config path (default ~/.orc/config.json)")
	fs.StringVar(&master, "master", "", "master provider override")
	fs.StringVar(&prompt, "prompt", "", "natural language flow prompt")
	fs.BoolVar(&asJSON, "json", false, "print result as JSON")

	if err := fs.Parse(args); err != nil {
		return 2
	}

	if strings.TrimSpace(prompt) == "" {
		prompt = strings.TrimSpace(strings.Join(fs.Args(), " "))
	}
	if strings.TrimSpace(prompt) == "" {
		fmt.Fprintln(errOut, "provide a natural-language flow prompt via --prompt or positional text")
		return 2
	}

	resolvedConfigPath, err := config.ResolvePath(configPath)
	if err != nil {
		fmt.Fprintln(errOut, err)
		return 1
	}

	availableProviders := make([]string, 0)
	cfg, cfgErr := config.Load(resolvedConfigPath)
	if cfgErr == nil {
		for _, provider := range cfg.Providers {
			if provider.Installed {
				availableProviders = append(availableProviders, normalizeProviderName(provider.Name))
			}
		}
		if strings.TrimSpace(master) == "" {
			master = cfg.MasterProvider
		}
	} else if !errors.Is(cfgErr, os.ErrNotExist) {
		fmt.Fprintln(errOut, cfgErr)
		return 1
	}

	master = normalizeProviderName(master)
	if master == "" {
		master = "codex"
	}

	definition, err := flow.Compile(flow.CompileOptions{
		Name:               name,
		Prompt:             prompt,
		MasterProvider:     master,
		AvailableProviders: availableProviders,
	})
	if err != nil {
		fmt.Fprintln(errOut, err)
		return 1
	}

	resolvedRepo, err := resolveRepoPath(repo)
	if err != nil {
		fmt.Fprintln(errOut, err)
		return 1
	}

	resolvedOutputPath := strings.TrimSpace(outputPath)
	if resolvedOutputPath == "" {
		resolvedOutputPath = filepath.Join(resolvedRepo, ".orc", "flows", definition.Name+".json")
	} else {
		resolvedOutputPath, err = resolveFilePath(resolvedOutputPath)
		if err != nil {
			fmt.Fprintln(errOut, err)
			return 1
		}
	}

	if err := flow.Save(resolvedOutputPath, definition); err != nil {
		fmt.Fprintln(errOut, err)
		return 1
	}

	result := flowCreateResult{
		FlowPath: resolvedOutputPath,
		Flow:     definition,
	}
	if asJSON {
		return writeJSON(out, errOut, result)
	}

	printFlowSummary(out, result)
	return 0
}

func runInvoke(args []string, out io.Writer, errOut io.Writer) int {
	fs := flag.NewFlagSet("invoke", flag.ContinueOnError)
	fs.SetOutput(errOut)

	var provider string
	var model string
	var message string
	var repo string
	var continueSession bool
	var timeoutSeconds int
	var asJSON bool

	fs.StringVar(&provider, "provider", "", "provider to invoke (claude|codex|opencode|kimi)")
	fs.StringVar(&model, "model", "", "model override")
	fs.StringVar(&message, "message", "", "message/prompt to send")
	fs.StringVar(&repo, "repo", "", "working directory (default current directory)")
	fs.BoolVar(&continueSession, "continue", false, "continue previous conversation/session")
	fs.IntVar(&timeoutSeconds, "timeout", 120, "timeout in seconds")
	fs.BoolVar(&asJSON, "json", false, "print result as JSON")

	if err := fs.Parse(args); err != nil {
		return 2
	}

	if strings.TrimSpace(message) == "" {
		message = strings.TrimSpace(strings.Join(fs.Args(), " "))
	}
	if strings.TrimSpace(message) == "" {
		fmt.Fprintln(errOut, "message is required (use --message or positional text)")
		return 2
	}

	workingDir, err := resolveRepoPath(repo)
	if err != nil {
		fmt.Fprintln(errOut, err)
		return 1
	}

	result, err := invoke.Execute(context.Background(), invoke.Options{
		Provider:        provider,
		Model:           model,
		Message:         message,
		WorkingDir:      workingDir,
		ContinueSession: continueSession,
		Timeout:         time.Duration(timeoutSeconds) * time.Second,
	})
	if err != nil {
		fmt.Fprintln(errOut, err)
		return 1
	}

	if asJSON {
		return writeJSON(out, errOut, result)
	}

	fmt.Fprintf(out, "Provider: %s\n", result.Provider)
	fmt.Fprintf(out, "Working Dir: %s\n", result.WorkingDir)
	fmt.Fprintf(out, "Duration: %dms\n", result.DurationMs)
	fmt.Fprintln(out, "Response:")
	fmt.Fprintln(out, result.Response)
	return 0
}

func runHandoff(args []string, out io.Writer, errOut io.Writer) int {
	fs := flag.NewFlagSet("handoff", flag.ContinueOnError)
	fs.SetOutput(errOut)

	var from string
	var to string
	var goal string
	var repo string
	var runsRoot string
	var asJSON bool
	acceptance := &stringListFlag{}

	fs.StringVar(&from, "from", "", "source agent")
	fs.StringVar(&to, "to", "", "target agent")
	fs.StringVar(&goal, "goal", "", "task goal")
	fs.StringVar(&repo, "repo", "", "repository path (default current directory)")
	fs.StringVar(&runsRoot, "runs-root", "", "run root directory (default <repo>/.orc/runs)")
	fs.BoolVar(&asJSON, "json", false, "print result as JSON")
	fs.Var(acceptance, "accept", "acceptance criterion (repeatable)")

	if err := fs.Parse(args); err != nil {
		return 2
	}

	svc := handoff.NewService()
	record, err := svc.Create(context.Background(), handoff.CreateOptions{
		From:               from,
		To:                 to,
		Goal:               goal,
		RepoPath:           repo,
		RunsRoot:           runsRoot,
		AcceptanceCriteria: acceptance.Values(),
	})
	if err != nil {
		fmt.Fprintln(errOut, err)
		return 1
	}

	if asJSON {
		return writeJSON(out, errOut, record)
	}

	printRunSummary(out, record)
	return 0
}

func runContinue(args []string, out io.Writer, errOut io.Writer) int {
	if len(args) == 0 {
		fmt.Fprintln(errOut, "usage: orc continue <run_id> --to <agent> [--goal <goal>] [--json]")
		return 2
	}

	runID := args[0]
	fs := flag.NewFlagSet("continue", flag.ContinueOnError)
	fs.SetOutput(errOut)

	var to string
	var goal string
	var repo string
	var runsRoot string
	var asJSON bool
	acceptance := &stringListFlag{}

	fs.StringVar(&to, "to", "", "target agent")
	fs.StringVar(&goal, "goal", "", "override goal")
	fs.StringVar(&repo, "repo", "", "repository path override")
	fs.StringVar(&runsRoot, "runs-root", "", "run root directory")
	fs.BoolVar(&asJSON, "json", false, "print result as JSON")
	fs.Var(acceptance, "accept", "acceptance criterion override (repeatable)")

	if err := fs.Parse(args[1:]); err != nil {
		return 2
	}

	svc := handoff.NewService()
	record, err := svc.Continue(context.Background(), handoff.ContinueOptions{
		RunID:              runID,
		To:                 to,
		Goal:               goal,
		RepoPath:           repo,
		RunsRoot:           runsRoot,
		AcceptanceCriteria: acceptance.Values(),
	})
	if err != nil {
		fmt.Fprintln(errOut, err)
		return 1
	}

	if asJSON {
		return writeJSON(out, errOut, record)
	}

	printRunSummary(out, record)
	return 0
}

func runStatus(args []string, out io.Writer, errOut io.Writer) int {
	if len(args) == 0 {
		fmt.Fprintln(errOut, "usage: orc status <run_id> [--json]")
		return 2
	}

	runID := args[0]
	fs := flag.NewFlagSet("status", flag.ContinueOnError)
	fs.SetOutput(errOut)

	var runsRoot string
	var asJSON bool
	fs.StringVar(&runsRoot, "runs-root", "", "run root directory")
	fs.BoolVar(&asJSON, "json", false, "print result as JSON")

	if err := fs.Parse(args[1:]); err != nil {
		return 2
	}

	svc := handoff.NewService()
	record, err := svc.Load(runsRoot, runID)
	if err != nil {
		fmt.Fprintln(errOut, err)
		return 1
	}

	if asJSON {
		return writeJSON(out, errOut, record)
	}

	printRunSummary(out, record)
	return 0
}

func printDoctorTable(out io.Writer, reports []providers.DoctorReport) {
	theme := tui.NewTheme(out)
	headers := []string{"STATUS", "PROVIDER", "AUTH", "VERSION", "CAPS", "BINARY", "NOTES"}
	rows := make([][]string, 0, len(reports))
	ready := 0
	for _, r := range reports {
		notes := strings.Join(r.Notes, "; ")
		if r.Error != "" {
			if notes != "" {
				notes += "; "
			}
			notes += r.Error
		}

		dot := doctorStatusDot(theme, r)
		if isProviderReady(r) {
			ready++
		}

		caps := fmt.Sprintf("non_interactive=%t json=%t", r.SupportsNonInteractive, r.SupportsJSON)
		rows = append(rows, []string{
			dot,
			r.Name,
			string(r.AuthStatus),
			nonEmptyOr(r.Version, "unknown"),
			caps,
			nonEmptyOr(r.Binary, "-"),
			nonEmptyOr(notes, "-"),
		})
	}

	fmt.Fprintln(out, tui.UnicodeTable(headers, rows))
	fmt.Fprintf(out, "%s %d/%d providers ready\n", theme.Dim("summary:"), ready, len(reports))
}

func doctorStatusDot(theme tui.Theme, report providers.DoctorReport) string {
	if isProviderReady(report) {
		return theme.Green("●")
	}
	if report.Installed {
		return theme.Yellow("●")
	}
	return theme.Red("●")
}

func isProviderReady(report providers.DoctorReport) bool {
	return report.Installed && report.AuthStatus == providers.AuthStatusLoggedIn
}

func printInitSummary(out io.Writer, result initResult) {
	fmt.Fprintf(out, "Config Path: %s\n", result.ConfigPath)
	fmt.Fprintf(out, "Master Provider: %s\n", result.MasterProvider)
	fmt.Fprintln(out, "Registered Providers:")
	for _, provider := range result.Providers {
		fmt.Fprintf(
			out,
			"  %s | auth=%s | json=%t | non_interactive=%t | %s\n",
			provider.Name,
			provider.AuthStatus,
			provider.SupportsJSON,
			provider.SupportsNonInteractive,
			nonEmptyOr(provider.Version, "unknown version"),
		)
	}
}

func printFlowSummary(out io.Writer, result flowCreateResult) {
	fmt.Fprintf(out, "Flow Path: %s\n", result.FlowPath)
	fmt.Fprintf(out, "Flow Name: %s\n", result.Flow.Name)
	fmt.Fprintf(out, "Master Provider: %s\n", result.Flow.MasterProvider)
	fmt.Fprintln(out, "Stages:")
	for _, stage := range result.Flow.Stages {
		detail := fmt.Sprintf("  %s | intent=%s | provider=%s", stage.ID, stage.Intent, stage.Provider)
		if stage.Profile != "" {
			detail += fmt.Sprintf(" | profile=%s", stage.Profile)
		}
		if stage.InputFrom != "" {
			detail += fmt.Sprintf(" | input_from=%s", stage.InputFrom)
		}
		fmt.Fprintln(out, detail)
	}
	if len(result.Flow.Warnings) > 0 {
		fmt.Fprintln(out, "Warnings:")
		for _, warning := range result.Flow.Warnings {
			fmt.Fprintf(out, "  %s\n", warning)
		}
	}
}

func printRunSummary(out io.Writer, record handoff.RunRecord) {
	fmt.Fprintf(out, "Run ID: %s\n", record.RunID)
	fmt.Fprintf(out, "Flow: %s -> %s\n", record.SourceAgent, record.TargetAgent)
	fmt.Fprintf(out, "Goal: %s\n", record.Goal)
	fmt.Fprintf(out, "Repo: %s\n", nonEmptyOr(record.Repo.Path, "-"))
	fmt.Fprintf(out, "Run Dir: %s\n", nonEmptyOr(record.RunDir, "-"))
	fmt.Fprintf(out, "Handoff: %s\n", nonEmptyOr(record.HandoffFile, "-"))
	fmt.Fprintf(out, "Brief: %s\n", nonEmptyOr(record.BriefFile, "-"))
	if record.PreviousRunID != "" {
		fmt.Fprintf(out, "Previous Run: %s\n", record.PreviousRunID)
	}
	if len(record.Warnings) > 0 {
		fmt.Fprintf(out, "Warnings: %s\n", strings.Join(record.Warnings, "; "))
	}
}

func writeJSON(out io.Writer, errOut io.Writer, value interface{}) int {
	enc := json.NewEncoder(out)
	enc.SetIndent("", "  ")
	if err := enc.Encode(value); err != nil {
		fmt.Fprintln(errOut, err)
		return 1
	}
	return 0
}

func normalizeProviderName(name string) string {
	value := strings.TrimSpace(strings.ToLower(name))
	switch value {
	case "claude code":
		return "claude"
	case "open code":
		return "opencode"
	default:
		return value
	}
}

func chooseMasterProvider(master string, providers []config.Provider) (string, error) {
	available := make(map[string]bool, len(providers))
	for _, provider := range providers {
		available[normalizeProviderName(provider.Name)] = true
	}

	requested := normalizeProviderName(master)
	if requested != "" {
		if !available[requested] {
			return "", fmt.Errorf("master provider %q is not installed/registered", requested)
		}
		return requested, nil
	}

	for _, candidate := range []string{"codex", "claude", "opencode", "kimi"} {
		if available[candidate] {
			return candidate, nil
		}
	}
	for candidate := range available {
		return candidate, nil
	}
	return "", fmt.Errorf("no providers available")
}

func resolveRepoPath(repo string) (string, error) {
	trimmed := strings.TrimSpace(repo)
	if trimmed == "" {
		cwd, err := os.Getwd()
		if err != nil {
			return "", fmt.Errorf("resolve current directory: %w", err)
		}
		return filepath.Clean(cwd), nil
	}
	absolute, err := filepath.Abs(trimmed)
	if err != nil {
		return "", fmt.Errorf("resolve repo path: %w", err)
	}
	return filepath.Clean(absolute), nil
}

func resolveFilePath(path string) (string, error) {
	trimmed := strings.TrimSpace(path)
	if trimmed == "" {
		return "", fmt.Errorf("file path is required")
	}
	if filepath.IsAbs(trimmed) {
		return filepath.Clean(trimmed), nil
	}
	cwd, err := os.Getwd()
	if err != nil {
		return "", fmt.Errorf("resolve current directory: %w", err)
	}
	return filepath.Join(cwd, trimmed), nil
}

func nonEmptyOr(value, fallback string) string {
	if strings.TrimSpace(value) == "" {
		return fallback
	}
	return value
}

func printRootHelp(out io.Writer) {
	fmt.Fprintln(out, "orc - local coding-agent handoff orchestrator")
	fmt.Fprintln(out)
	fmt.Fprintln(out, "Running `orc` with no subcommand starts interactive chat mode.")
	fmt.Fprintln(out)
	fmt.Fprintln(out, "Usage:")
	fmt.Fprintln(out, "  orc [--flow <name>] [--repo <path>] [--once <message>] [--timeout <sec>] [--continue=<bool>] [--strict-scope=<bool>] [--stream=<bool>] [--live=<bool>] [--verbose=<bool>] [--show-raw=<bool>] [--json] [--max-output-chars <n>]")
	fmt.Fprintln(out, "  orc init [--interactive] [--master <provider>] [--json]")
	fmt.Fprintln(out, "  orc providers doctor [--provider <name>] [--json]")
	fmt.Fprintln(out, "  orc flow create [--name <name>] \"<natural language flow>\" [--json]")
	fmt.Fprintln(out, "  orc chat [--flow <name>] [--repo <path>] [--once <message>] [--strict-scope=<bool>] [--stream=<bool>] [--live=<bool>] [--verbose=<bool>] [--show-raw=<bool>] [--json] [--max-output-chars <n>]")
	fmt.Fprintln(out, "  orc invoke --provider <claude|codex|opencode|kimi> --message <text> [--continue] [--timeout <sec>] [--json]")
	fmt.Fprintln(out, "  orc handoff --from <agent> --to <agent> --goal <goal> [--accept <criterion>] [--json]")
	fmt.Fprintln(out, "  orc continue <run_id> --to <agent> [--goal <goal>] [--json]")
	fmt.Fprintln(out, "  orc status <run_id> [--json]")
}

func printProvidersHelp(out io.Writer) {
	fmt.Fprintln(out, "Usage:")
	fmt.Fprintln(out, "  orc providers doctor [--provider <name>] [--json]")
}

func printFlowHelp(out io.Writer) {
	fmt.Fprintln(out, "Usage:")
	fmt.Fprintln(out, "  orc flow create [--name <name>] [--master <provider>] [--json] \"<natural language flow>\"")
}

type stringListFlag struct {
	values []string
}

func (f *stringListFlag) String() string {
	return strings.Join(f.values, ",")
}

func (f *stringListFlag) Set(value string) error {
	trimmed := strings.TrimSpace(value)
	if trimmed == "" {
		return nil
	}
	f.values = append(f.values, trimmed)
	return nil
}

func (f *stringListFlag) Values() []string {
	if len(f.values) == 0 {
		return nil
	}
	out := make([]string, len(f.values))
	copy(out, f.values)
	return out
}
