import { test } from "node:test";
import assert from "node:assert/strict";
import { add, double } from "../math.js";

test("adds two numbers", () => {
  assert.equal(add(1, 2), 3);
});

test("doubles a number", () => {
  assert.equal(double(2), 4);
});
