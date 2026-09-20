import { expect, test } from "vitest";
import { add, double } from "./math.js";

test("adds two numbers", () => {
  expect(add(1, 2)).toBe(3);
});

test("doubles a number", () => {
  expect(double(2)).toBe(4);
});
