// Package parse is a tiny parser with a deliberate bug for the fixture.
package parse

import "strings"

// First returns the first comma-separated field, trimmed.
func First(s string) string {
	fields := strings.Split(s, ",")
	if len(fields) == 0 {
		return "x"
	}
	return strings.TrimSpace(fields[0])
}
