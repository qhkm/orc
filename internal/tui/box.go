package tui

import (
	"regexp"
	"strings"
	"unicode/utf8"
)

var ansiPattern = regexp.MustCompile(`\x1b\[[0-9;]*m`)

func Box(title string, lines []string) string {
	content := flattenLines(lines)
	width := visibleWidth(title)
	for _, line := range content {
		if w := visibleWidth(line); w > width {
			width = w
		}
	}
	if width < 1 {
		width = 1
	}

	var b strings.Builder
	b.WriteString(topBorder(title, width))
	b.WriteString("\n")
	for _, line := range content {
		pad := width - visibleWidth(line)
		if pad < 0 {
			pad = 0
		}
		b.WriteString("│ ")
		b.WriteString(line)
		b.WriteString(strings.Repeat(" ", pad))
		b.WriteString(" │\n")
	}
	if len(content) == 0 {
		b.WriteString("│ ")
		b.WriteString(strings.Repeat(" ", width))
		b.WriteString(" │\n")
	}
	b.WriteString("└")
	b.WriteString(strings.Repeat("─", width+2))
	b.WriteString("┘")
	return b.String()
}

func topBorder(title string, width int) string {
	trimmed := strings.TrimSpace(title)
	if trimmed == "" {
		return "┌" + strings.Repeat("─", width+2) + "┐"
	}
	titleLen := visibleWidth(trimmed)
	remaining := width - titleLen
	if remaining < 0 {
		remaining = 0
	}
	return "┌─ " + trimmed + " " + strings.Repeat("─", remaining) + "┐"
}

func flattenLines(lines []string) []string {
	if len(lines) == 0 {
		return nil
	}
	out := make([]string, 0, len(lines))
	for _, line := range lines {
		parts := strings.Split(strings.ReplaceAll(line, "\r\n", "\n"), "\n")
		out = append(out, parts...)
	}
	return out
}

func visibleWidth(value string) int {
	clean := ansiPattern.ReplaceAllString(value, "")
	return utf8.RuneCountInString(clean)
}
