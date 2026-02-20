package cli

import (
	"bufio"
	"errors"
	"fmt"
	"io"
	"os"
	"strconv"
	"strings"
	"text/tabwriter"

	"github.com/drnoraniza/orc/internal/config"
)

var errInitCanceled = errors.New("init canceled")

type initWizardResult struct {
	Providers      []config.Provider
	MasterProvider string
	ConfigPath     string
}

type wizardTheme struct {
	color bool
}

func newWizardTheme() wizardTheme {
	term := strings.TrimSpace(os.Getenv("TERM"))
	if term == "" || term == "dumb" || strings.TrimSpace(os.Getenv("NO_COLOR")) != "" {
		return wizardTheme{color: false}
	}
	return wizardTheme{color: true}
}

func (t wizardTheme) apply(value, code string) string {
	if !t.color {
		return value
	}
	return "\x1b[" + code + "m" + value + "\x1b[0m"
}

func (t wizardTheme) title(value string) string {
	return t.apply(value, "1;36")
}

func (t wizardTheme) section(value string) string {
	return t.apply(value, "1")
}

func (t wizardTheme) muted(value string) string {
	return t.apply(value, "2")
}

func (t wizardTheme) prompt(value string) string {
	return t.apply(value, "1;32")
}

func (t wizardTheme) warn(value string) string {
	return t.apply(value, "1;33")
}

func (t wizardTheme) ok(value string) string {
	return t.apply(value, "1;32")
}

func runInitWizard(in io.Reader, out io.Writer, detected []config.Provider, requestedMaster, defaultConfigPath string) (initWizardResult, error) {
	theme := newWizardTheme()
	reader := bufio.NewReader(in)

	printDivider(out, 70)
	fmt.Fprintln(out, theme.title("ORC INIT WIZARD"))
	fmt.Fprintln(out, theme.muted("Configure provider registration and choose a master CLI."))
	printDivider(out, 70)
	fmt.Fprintln(out)
	fmt.Fprintln(out, theme.section("Detected providers"))
	printWizardProviderTable(out, detected, theme)

	registerAll, err := promptYesNo(reader, out, theme.prompt("Register all detected providers? [Y/n]: "), true, theme)
	if err != nil {
		return initWizardResult{}, err
	}

	selected := detected
	if !registerAll {
		for {
			selectionRaw, err := promptLine(reader, out, theme.prompt("Select providers (e.g. 1,3 or codex,kimi): "))
			if err != nil {
				return initWizardResult{}, err
			}
			selected, err = parseProviderSelection(selectionRaw, detected)
			if err == nil {
				break
			}
			fmt.Fprintf(out, "%s %v\n", theme.warn("Invalid selection:"), err)
		}
	}
	if len(selected) == 0 {
		return initWizardResult{}, fmt.Errorf("no providers selected")
	}

	selectedMaster, err := pickMasterProvider(reader, out, selected, requestedMaster, theme)
	if err != nil {
		return initWizardResult{}, err
	}

	configPathInput, err := promptLine(reader, out, theme.prompt(fmt.Sprintf("Config path [%s]: ", defaultConfigPath)))
	if err != nil {
		return initWizardResult{}, err
	}
	configPath := defaultConfigPath
	if strings.TrimSpace(configPathInput) != "" {
		configPath = strings.TrimSpace(configPathInput)
	}

	fmt.Fprintln(out)
	fmt.Fprintln(out, theme.section("Summary"))
	fmt.Fprintf(out, "- Providers: %s\n", providerNames(selected))
	fmt.Fprintf(out, "- Master: %s\n", selectedMaster)
	fmt.Fprintf(out, "- Config: %s\n", configPath)

	confirm, err := promptYesNo(reader, out, theme.prompt("Save this configuration? [Y/n]: "), true, theme)
	if err != nil {
		return initWizardResult{}, err
	}
	if !confirm {
		return initWizardResult{}, errInitCanceled
	}

	fmt.Fprintln(out, theme.ok("Configuration saved."))
	return initWizardResult{
		Providers:      selected,
		MasterProvider: selectedMaster,
		ConfigPath:     configPath,
	}, nil
}

func pickMasterProvider(reader *bufio.Reader, out io.Writer, selected []config.Provider, requestedMaster string, theme wizardTheme) (string, error) {
	requested := normalizeProviderName(requestedMaster)
	if requested != "" {
		for _, provider := range selected {
			if normalizeProviderName(provider.Name) == requested {
				return requested, nil
			}
		}
		return "", fmt.Errorf("master provider %q is not in selected providers", requested)
	}

	defaultMaster, err := chooseMasterProvider("", selected)
	if err != nil {
		return "", err
	}
	defaultIndex := 1
	for i, provider := range selected {
		if normalizeProviderName(provider.Name) == defaultMaster {
			defaultIndex = i + 1
			break
		}
	}

	fmt.Fprintln(out)
	fmt.Fprintln(out, theme.section("Master provider"))
	for i, provider := range selected {
		fmt.Fprintf(out, "%d) %s\n", i+1, normalizeProviderName(provider.Name))
	}

	for {
		masterInput, err := promptLine(reader, out, theme.prompt(fmt.Sprintf("Choose master provider [%d]: ", defaultIndex)))
		if err != nil {
			return "", err
		}
		if strings.TrimSpace(masterInput) == "" {
			return defaultMaster, nil
		}

		token := strings.TrimSpace(masterInput)
		if idx, err := strconv.Atoi(token); err == nil {
			if idx < 1 || idx > len(selected) {
				fmt.Fprintf(out, "%s index %d is out of range\n", theme.warn("Invalid master selection:"), idx)
				continue
			}
			return normalizeProviderName(selected[idx-1].Name), nil
		}

		name := normalizeProviderName(token)
		for _, provider := range selected {
			if normalizeProviderName(provider.Name) == name {
				return name, nil
			}
		}
		fmt.Fprintf(out, "%s unknown provider %q\n", theme.warn("Invalid master selection:"), token)
	}
}

func printWizardProviderTable(out io.Writer, providers []config.Provider, theme wizardTheme) {
	tw := tabwriter.NewWriter(out, 2, 2, 2, ' ', 0)
	fmt.Fprintln(tw, "IDX\tPROVIDER\tSTATUS\tAUTH\tJSON\tNON_INTERACTIVE\tVERSION")
	for i, provider := range providers {
		fmt.Fprintf(
			tw,
			"%d\t%s\t%s\t%s\t%t\t%t\t%s\n",
			i+1,
			normalizeProviderName(provider.Name),
			providerStatus(provider, theme),
			nonEmptyOr(provider.AuthStatus, "unknown"),
			provider.SupportsJSON,
			provider.SupportsNonInteractive,
			nonEmptyOr(provider.Version, "unknown"),
		)
	}
	_ = tw.Flush()
}

func providerStatus(provider config.Provider, theme wizardTheme) string {
	auth := strings.ToLower(strings.TrimSpace(provider.AuthStatus))
	switch {
	case auth == "logged_in" && provider.SupportsJSON && provider.SupportsNonInteractive:
		return theme.ok("ready")
	case auth == "logged_out":
		return theme.warn("login")
	default:
		return theme.warn("check")
	}
}

func promptLine(reader *bufio.Reader, out io.Writer, prompt string) (string, error) {
	fmt.Fprint(out, prompt)
	line, err := reader.ReadString('\n')
	if err != nil {
		if errors.Is(err, io.EOF) {
			return strings.TrimSpace(line), nil
		}
		return "", err
	}
	return strings.TrimSpace(line), nil
}

func promptYesNo(reader *bufio.Reader, out io.Writer, prompt string, defaultYes bool, theme wizardTheme) (bool, error) {
	for {
		line, err := promptLine(reader, out, prompt)
		if err != nil {
			return false, err
		}
		if strings.TrimSpace(line) == "" {
			return defaultYes, nil
		}

		switch strings.ToLower(strings.TrimSpace(line)) {
		case "y", "yes":
			return true, nil
		case "n", "no":
			return false, nil
		default:
			fmt.Fprintf(out, "%s please answer y or n\n", theme.warn("Invalid response:"))
		}
	}
}

func parseProviderSelection(input string, detected []config.Provider) ([]config.Provider, error) {
	tokens := strings.Split(input, ",")
	selected := make([]config.Provider, 0, len(tokens))
	seen := make(map[string]bool)

	for _, token := range tokens {
		trimmed := strings.TrimSpace(token)
		if trimmed == "" {
			continue
		}

		var provider config.Provider
		found := false
		if idx, err := strconv.Atoi(trimmed); err == nil {
			if idx < 1 || idx > len(detected) {
				return nil, fmt.Errorf("provider index out of range: %d", idx)
			}
			provider = detected[idx-1]
			found = true
		} else {
			name := normalizeProviderName(trimmed)
			for _, candidate := range detected {
				if normalizeProviderName(candidate.Name) == name {
					provider = candidate
					found = true
					break
				}
			}
		}

		if !found {
			return nil, fmt.Errorf("unknown provider selection: %s", trimmed)
		}

		providerKey := normalizeProviderName(provider.Name)
		if seen[providerKey] {
			continue
		}
		seen[providerKey] = true
		selected = append(selected, provider)
	}

	if len(selected) == 0 {
		return nil, fmt.Errorf("no providers selected")
	}

	return selected, nil
}

func providerNames(providers []config.Provider) string {
	names := make([]string, 0, len(providers))
	for _, provider := range providers {
		names = append(names, normalizeProviderName(provider.Name))
	}
	return strings.Join(names, ", ")
}

func printDivider(out io.Writer, width int) {
	if width < 1 {
		width = 40
	}
	fmt.Fprintln(out, strings.Repeat("=", width))
}
