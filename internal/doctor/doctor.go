package doctor

import (
	"context"
	"fmt"
	"strings"
	"sync"

	"github.com/drnoraniza/orc/internal/providers"
)

type Options struct {
	Provider string
}

func Run(ctx context.Context, registry []providers.Provider, opts Options) ([]providers.DoctorReport, error) {
	selected := make([]providers.Provider, 0, len(registry))
	for _, provider := range registry {
		if opts.Provider == "" || strings.EqualFold(provider.Name(), opts.Provider) {
			selected = append(selected, provider)
		}
	}

	if len(selected) == 0 {
		return nil, fmt.Errorf("provider %q not found", opts.Provider)
	}

	reports := make([]providers.DoctorReport, len(selected))
	var wg sync.WaitGroup

	for i, provider := range selected {
		wg.Add(1)
		idx := i
		p := provider
		go func() {
			defer wg.Done()
			reports[idx] = p.Doctor(ctx)
		}()
	}

	wg.Wait()
	return reports, nil
}
