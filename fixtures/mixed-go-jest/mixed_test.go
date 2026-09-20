package mixed

import "testing"

func TestGreet(t *testing.T) {
	if got := Greet("x"); got != "hello x" {
		t.Errorf("got %q, want %q", got, "hello x")
	}
}
