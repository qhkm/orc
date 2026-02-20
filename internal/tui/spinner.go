package tui

import (
	"fmt"
	"io"
	"time"
)

const ansiClearLine = "\x1b[2K"

type ProgressLine struct {
	out     io.Writer
	theme   Theme
	enabled bool
	frames  []string
	frame   int
}

func NewProgressLine(out io.Writer, theme Theme, enabled bool) ProgressLine {
	return ProgressLine{
		out:     out,
		theme:   theme,
		enabled: enabled,
		frames:  []string{"-", "\\", "|", "/"},
	}
}

func (p *ProgressLine) Tick(stageNum, stageTotal int, intent, provider string, elapsed time.Duration) {
	if !p.enabled {
		return
	}
	frame := p.frames[p.frame%len(p.frames)]
	p.frame++
	badges := IntentProviderBadge(p.theme, intent, provider)
	fmt.Fprintf(p.out, "\r%s %s %s elapsed=%s", frame, p.theme.Dim(fmt.Sprintf("[%d/%d]", stageNum, stageTotal)), badges, elapsed.Round(100*time.Millisecond))
}

func (p *ProgressLine) Stop(stageNum, stageTotal int, intent, provider string, elapsed time.Duration, err error) {
	if !p.enabled {
		return
	}
	status := p.theme.Green("done \u2713")
	if err != nil {
		status = p.theme.Red("fail \u2717")
	}
	badges := IntentProviderBadge(p.theme, intent, provider)
	fmt.Fprintf(p.out, "\r%s\r%s %s %s elapsed=%s\n", ansiClearLine, p.theme.Dim(fmt.Sprintf("[%d/%d]", stageNum, stageTotal)), badges, status, elapsed.Round(100*time.Millisecond))
}
