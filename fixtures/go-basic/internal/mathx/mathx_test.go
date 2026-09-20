package mathx

import "testing"

func TestAdd(t *testing.T) {
	if Add(1, 2) != 3 {
		t.Fatal("1+2 != 3")
	}
}

func TestSkipped(t *testing.T) {
	t.Skip("fixture: one skipped test")
}
