package tui

import (
	"io"
	"os"
	"strings"
)

const (
	ansiReset   = "\x1b[0m"
	ansiBold    = "\x1b[1m"
	ansiDim     = "\x1b[2m"
	ansiRed     = "\x1b[31m"
	ansiGreen   = "\x1b[32m"
	ansiYellow  = "\x1b[33m"
	ansiBlue    = "\x1b[34m"
	ansiMagenta = "\x1b[35m"
	ansiCyan    = "\x1b[36m"
	ansiPurple  = "\x1b[38;5;141m"
	ansiOrange  = "\x1b[38;5;208m"
)

type Theme struct {
	Color bool
}

func NewTheme(out io.Writer) Theme {
	if strings.TrimSpace(os.Getenv("NO_COLOR")) != "" {
		return Theme{Color: false}
	}
	if strings.EqualFold(strings.TrimSpace(os.Getenv("ORC_NO_COLOR")), "1") {
		return Theme{Color: false}
	}
	if strings.EqualFold(strings.TrimSpace(os.Getenv("TERM")), "dumb") {
		return Theme{Color: false}
	}
	return Theme{Color: IsTerminalWriter(out)}
}

func IsTerminalWriter(out io.Writer) bool {
	file, ok := out.(*os.File)
	if !ok {
		return false
	}
	info, err := file.Stat()
	if err != nil {
		return false
	}
	return (info.Mode() & os.ModeCharDevice) != 0
}

func (t Theme) Paint(code, value string) string {
	if !t.Color || strings.TrimSpace(code) == "" || value == "" {
		return value
	}
	return code + value + ansiReset
}

func (t Theme) Bold(value string) string {
	return t.Paint(ansiBold, value)
}

func (t Theme) Dim(value string) string {
	return t.Paint(ansiDim, value)
}

func (t Theme) Red(value string) string {
	return t.Paint(ansiRed, value)
}

func (t Theme) Green(value string) string {
	return t.Paint(ansiGreen, value)
}

func (t Theme) Yellow(value string) string {
	return t.Paint(ansiYellow, value)
}

func (t Theme) Blue(value string) string {
	return t.Paint(ansiBlue, value)
}

func (t Theme) Magenta(value string) string {
	return t.Paint(ansiMagenta, value)
}

func (t Theme) Cyan(value string) string {
	return t.Paint(ansiCyan, value)
}

func (t Theme) Purple(value string) string {
	return t.Paint(ansiPurple, value)
}

func (t Theme) Orange(value string) string {
	return t.Paint(ansiOrange, value)
}

func (t Theme) Prompt(actor string) string {
	prefix := strings.TrimSpace(actor)
	if prefix == "" {
		prefix = "you"
	}
	return t.Green(prefix + "> ")
}

func Rule(symbol string, width int) string {
	if symbol == "" {
		symbol = "-"
	}
	if width <= 0 {
		width = 72
	}
	return strings.Repeat(symbol, width)
}
