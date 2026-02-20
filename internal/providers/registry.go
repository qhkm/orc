package providers

func DefaultRegistry() []Provider {
	return []Provider{
		NewCodexProvider(),
		NewGenericProvider("claude", "claude", [][]string{{"--version"}, {"version"}}, []string{"--help"}, "print"),
		NewGenericProvider("opencode", "opencode", [][]string{{"--version"}, {"version"}}, []string{"--help"}, "run"),
		NewGenericProvider("kimi", "kimi", [][]string{{"--version"}, {"version"}}, []string{"--help"}, "non-interactive"),
	}
}
