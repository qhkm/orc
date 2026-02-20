package tui

import "strings"

func IntentBadge(theme Theme, intent string) string {
	name := normalizeBadge(intent, "task")
	return "[" + colorIntent(theme, name) + "]"
}

func ProviderBadge(theme Theme, provider string) string {
	name := normalizeBadge(provider, "unknown")
	return "[" + colorProvider(theme, name) + "]"
}

func IntentProviderBadge(theme Theme, intent, provider string) string {
	intentName := normalizeBadge(intent, "task")
	providerName := normalizeBadge(provider, "unknown")
	return "[" + colorIntent(theme, intentName) + "·" + colorProvider(theme, providerName) + "]"
}

func colorIntent(theme Theme, intent string) string {
	switch intent {
	case "brainstorm":
		return theme.Blue(intent)
	case "implement":
		return theme.Green(intent)
	case "cross_check", "cross-check":
		return theme.Yellow(intent)
	case "review":
		return theme.Magenta(intent)
	default:
		return theme.Cyan(intent)
	}
}

func colorProvider(theme Theme, provider string) string {
	switch provider {
	case "claude":
		return theme.Purple(provider)
	case "codex":
		return theme.Cyan(provider)
	case "kimi":
		return theme.Orange(provider)
	case "opencode":
		return theme.Blue(provider)
	default:
		return theme.Dim(provider)
	}
}

func normalizeBadge(value, fallback string) string {
	trimmed := strings.TrimSpace(strings.ToLower(value))
	if trimmed == "" {
		return fallback
	}
	return trimmed
}
