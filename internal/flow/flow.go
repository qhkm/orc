package flow

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"regexp"
	"sort"
	"strings"
	"time"
)

const maxIntentAliasDistance = 120

type Stage struct {
	ID        string `json:"id"`
	Intent    string `json:"intent"`
	Provider  string `json:"provider"`
	Profile   string `json:"profile,omitempty"`
	InputFrom string `json:"input_from,omitempty"`
}

type Definition struct {
	Name           string    `json:"name"`
	SourcePrompt   string    `json:"source_prompt"`
	MasterProvider string    `json:"master_provider"`
	Stages         []Stage   `json:"stages"`
	Warnings       []string  `json:"warnings,omitempty"`
	CreatedAt      time.Time `json:"created_at"`
}

type CompileOptions struct {
	Name               string
	Prompt             string
	MasterProvider     string
	AvailableProviders []string
	Now                func() time.Time
}

type intentRule struct {
	ID              string
	Keywords        []string
	DefaultProvider string
}

var (
	rules = []intentRule{
		{ID: "brainstorm", Keywords: []string{"brainstorm", "brainstorming", "ideate", "planning", "plan"}, DefaultProvider: "claude"},
		{ID: "implement", Keywords: []string{"implement", "implementation", "build", "develop", "write code", "coding"}, DefaultProvider: "codex"},
		{ID: "cross_check", Keywords: []string{"cross check", "cross-check", "double check", "validate", "verify"}, DefaultProvider: "kimi"},
		{ID: "review", Keywords: []string{"review", "audit", "critique"}, DefaultProvider: "claude"},
	}
	providerAliases = map[string][]string{
		"claude":   {"claude code", "claude"},
		"codex":    {"codex"},
		"opencode": {"opencode", "open code"},
		"kimi":     {"kimi"},
	}
	providerOrder = []string{"claude", "codex", "kimi", "opencode"}
)

func Compile(opts CompileOptions) (Definition, error) {
	prompt := strings.TrimSpace(opts.Prompt)
	if prompt == "" {
		return Definition{}, fmt.Errorf("prompt is required")
	}

	master := normalizeProvider(opts.MasterProvider)
	available := normalizeProviderSet(opts.AvailableProviders)
	if master == "" {
		master = pickAvailableOrDefault(available, []string{"codex", "claude", "opencode", "kimi"}, "codex")
	}

	lowerPrompt := strings.ToLower(prompt)
	stages := make([]Stage, 0)
	warnings := make([]string, 0)

	for _, rule := range rules {
		if !containsAny(lowerPrompt, rule.Keywords) {
			continue
		}

		provider, profile := providerForIntent(lowerPrompt, rule, master, available)
		if provider == "" {
			provider = master
		}

		stageID := fmt.Sprintf("%s_%d", rule.ID, len(stages)+1)
		stage := Stage{
			ID:       stageID,
			Intent:   rule.ID,
			Provider: provider,
			Profile:  profile,
		}
		if len(stages) > 0 {
			stage.InputFrom = stages[len(stages)-1].ID
		}
		stages = append(stages, stage)

		if len(available) > 0 && !available[provider] {
			warnings = append(warnings, fmt.Sprintf("provider %q for stage %q is not registered in config", provider, rule.ID))
		}
	}

	if len(stages) == 0 {
		provider := master
		profile := ""
		if strings.Contains(lowerPrompt, "opus") {
			provider = "claude"
			profile = "opus"
		}
		stages = append(stages, Stage{
			ID:       "task_1",
			Intent:   "task",
			Provider: provider,
			Profile:  profile,
		})
		if len(available) > 0 && !available[provider] {
			warnings = append(warnings, fmt.Sprintf("provider %q is not registered in config", provider))
		}
	}

	nowFn := opts.Now
	if nowFn == nil {
		nowFn = time.Now
	}

	name := NormalizeName(opts.Name)
	if name == "" {
		name = "default_flow"
	}

	sort.Strings(warnings)
	return Definition{
		Name:           name,
		SourcePrompt:   prompt,
		MasterProvider: master,
		Stages:         stages,
		Warnings:       warnings,
		CreatedAt:      nowFn().UTC(),
	}, nil
}

func Save(path string, def Definition) error {
	resolvedPath, err := resolveOutputPath(path)
	if err != nil {
		return err
	}
	if err := os.MkdirAll(filepath.Dir(resolvedPath), 0o755); err != nil {
		return fmt.Errorf("create flow directory: %w", err)
	}

	payload, err := json.MarshalIndent(def, "", "  ")
	if err != nil {
		return fmt.Errorf("marshal flow: %w", err)
	}
	if err := os.WriteFile(resolvedPath, payload, 0o644); err != nil {
		return fmt.Errorf("write flow: %w", err)
	}
	return nil
}

func Load(path string) (Definition, error) {
	resolvedPath, err := resolveInputPath(path)
	if err != nil {
		return Definition{}, err
	}
	payload, err := os.ReadFile(resolvedPath)
	if err != nil {
		return Definition{}, fmt.Errorf("read flow: %w", err)
	}

	var def Definition
	if err := json.Unmarshal(payload, &def); err != nil {
		return Definition{}, fmt.Errorf("parse flow: %w", err)
	}
	if strings.TrimSpace(def.Name) == "" {
		return Definition{}, fmt.Errorf("flow name is required")
	}
	if len(def.Stages) == 0 {
		return Definition{}, fmt.Errorf("flow must include at least one stage")
	}
	return def, nil
}

func NormalizeName(name string) string {
	trimmed := strings.TrimSpace(strings.ToLower(name))
	if trimmed == "" {
		return ""
	}
	reg := regexp.MustCompile(`[^a-z0-9]+`)
	normalized := reg.ReplaceAllString(trimmed, "_")
	normalized = strings.Trim(normalized, "_")
	return normalized
}

func providerForIntent(prompt string, rule intentRule, master string, available map[string]bool) (string, string) {
	if rule.ID == "review" && termsNear(prompt, rule.Keywords, "opus", maxIntentAliasDistance) {
		return "claude", "opus"
	}

	if provider := bestProviderForKeywords(prompt, rule.Keywords); provider != "" {
		profile := ""
		if rule.ID == "review" && strings.Contains(prompt, "opus") && provider == "claude" {
			profile = "opus"
		}
		return provider, profile
	}

	defaultProvider := normalizeProvider(rule.DefaultProvider)
	if len(available) > 0 && !available[defaultProvider] {
		if available[master] {
			defaultProvider = master
		} else {
			defaultProvider = pickAvailableOrDefault(available, providerOrder, defaultProvider)
		}
	}

	profile := ""
	if rule.ID == "review" && strings.Contains(prompt, "opus") && defaultProvider == "claude" {
		profile = "opus"
	}
	return defaultProvider, profile
}

func bestProviderForKeywords(prompt string, keywords []string) string {
	bestProvider := ""
	bestDistance := maxIntentAliasDistance + 1

	for _, provider := range providerOrder {
		aliases := providerAliases[provider]
		for _, alias := range aliases {
			for _, keyword := range keywords {
				distance, ok := minDistanceBetweenTerms(prompt, alias, keyword)
				if !ok {
					continue
				}
				if distance <= maxIntentAliasDistance && distance < bestDistance {
					bestDistance = distance
					bestProvider = provider
				}
			}
		}
	}

	return bestProvider
}

func termsNear(prompt string, keywords []string, alias string, maxDistance int) bool {
	for _, keyword := range keywords {
		distance, ok := minDistanceBetweenTerms(prompt, alias, keyword)
		if ok && distance <= maxDistance {
			return true
		}
	}
	return false
}

func minDistanceBetweenTerms(text, first, second string) (int, bool) {
	firstPositions := indicesForTerm(text, first)
	if len(firstPositions) == 0 {
		return 0, false
	}
	secondPositions := indicesForTerm(text, second)
	if len(secondPositions) == 0 {
		return 0, false
	}

	best := -1
	for _, a := range firstPositions {
		for _, b := range secondPositions {
			distance := a - b
			if distance < 0 {
				distance = -distance
			}
			if best == -1 || distance < best {
				best = distance
			}
		}
	}
	if best < 0 {
		return 0, false
	}
	return best, true
}

func indicesForTerm(text, term string) []int {
	normalizedTerm := strings.TrimSpace(strings.ToLower(term))
	if normalizedTerm == "" {
		return nil
	}
	pattern := fmt.Sprintf(`\b%s\b`, regexp.QuoteMeta(normalizedTerm))
	re := regexp.MustCompile(pattern)
	locs := re.FindAllStringIndex(text, -1)
	if len(locs) == 0 {
		return nil
	}
	positions := make([]int, 0, len(locs))
	for _, loc := range locs {
		positions = append(positions, loc[0])
	}
	return positions
}

func normalizeProviderSet(values []string) map[string]bool {
	out := make(map[string]bool, len(values))
	for _, value := range values {
		normalized := normalizeProvider(value)
		if normalized == "" {
			continue
		}
		out[normalized] = true
	}
	return out
}

func normalizeProvider(value string) string {
	trimmed := strings.TrimSpace(strings.ToLower(value))
	switch trimmed {
	case "claude code":
		return "claude"
	case "open code":
		return "opencode"
	default:
		return trimmed
	}
}

func containsAny(prompt string, keywords []string) bool {
	for _, keyword := range keywords {
		if strings.Contains(prompt, strings.ToLower(keyword)) {
			return true
		}
	}
	return false
}

func pickAvailableOrDefault(available map[string]bool, preferred []string, fallback string) string {
	if len(available) == 0 {
		if fallback != "" {
			return fallback
		}
		return preferred[0]
	}
	for _, provider := range preferred {
		if available[provider] {
			return provider
		}
	}
	if available[fallback] {
		return fallback
	}
	for provider := range available {
		return provider
	}
	return fallback
}

func resolveOutputPath(path string) (string, error) {
	trimmed := strings.TrimSpace(path)
	if trimmed == "" {
		return "", fmt.Errorf("output path is required")
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

func resolveInputPath(path string) (string, error) {
	trimmed := strings.TrimSpace(path)
	if trimmed == "" {
		return "", fmt.Errorf("flow path is required")
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
