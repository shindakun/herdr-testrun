package parse

import "testing"

func TestFirstOK(t *testing.T) {
	if got := First(" a , b"); got != "a" {
		t.Fatalf("got %q, want %q", got, "a")
	}
}

func TestFirstFails(t *testing.T) {
	if got := First("a,b"); got != "b" {
		t.Errorf("got %q, want %q", got, "b")
	}
}

func TestParse(t *testing.T) {
	t.Run("plain", func(t *testing.T) {
		if got := First("a"); got != "a" {
			t.Errorf("got %q, want %q", got, "a")
		}
	})
	t.Run("empty_input", func(t *testing.T) {
		if got := First(""); got != "x" {
			t.Errorf("got %q, want %q", got, "x")
		}
	})
}
