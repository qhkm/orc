package providers

import "context"

type AuthStatus string

const (
	AuthStatusUnknown   AuthStatus = "unknown"
	AuthStatusLoggedIn  AuthStatus = "logged_in"
	AuthStatusLoggedOut AuthStatus = "logged_out"
	AuthStatusError     AuthStatus = "error"
)

type DoctorReport struct {
	Name                   string     `json:"name"`
	Binary                 string     `json:"binary"`
	Installed              bool       `json:"installed"`
	Version                string     `json:"version,omitempty"`
	AuthStatus             AuthStatus `json:"auth_status"`
	SupportsNonInteractive bool       `json:"supports_non_interactive"`
	SupportsJSON           bool       `json:"supports_json"`
	Notes                  []string   `json:"notes,omitempty"`
	Error                  string     `json:"error,omitempty"`
}

type Provider interface {
	Name() string
	Doctor(ctx context.Context) DoctorReport
}
