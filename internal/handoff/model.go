package handoff

import "time"

type RepoState struct {
	Path        string   `json:"path"`
	Branch      string   `json:"branch,omitempty"`
	BaseCommit  string   `json:"base_commit,omitempty"`
	Dirty       bool     `json:"dirty"`
	StatusShort []string `json:"status_short,omitempty"`
}

type RunRecord struct {
	RunID              string    `json:"run_id"`
	SourceAgent        string    `json:"source_agent"`
	TargetAgent        string    `json:"target_agent"`
	Goal               string    `json:"goal"`
	AcceptanceCriteria []string  `json:"acceptance_criteria,omitempty"`
	Repo               RepoState `json:"repo"`
	PreviousRunID      string    `json:"previous_run_id,omitempty"`
	Status             string    `json:"status"`
	CreatedAt          time.Time `json:"created_at"`
	UpdatedAt          time.Time `json:"updated_at"`
	RunDir             string    `json:"run_dir"`
	HandoffFile        string    `json:"handoff_file"`
	BriefFile          string    `json:"brief_file"`
	Warnings           []string  `json:"warnings,omitempty"`
}
