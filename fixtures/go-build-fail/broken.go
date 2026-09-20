// Package gobuildfail does not compile, on purpose.
package gobuildfail

// Broken has a return value its signature does not declare.
func Broken() { return 1 }
