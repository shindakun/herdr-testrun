package gobuildfail

import "testing"

func TestNeverRuns(t *testing.T) {
	Broken()
}
