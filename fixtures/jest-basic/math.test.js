const { add, double } = require("./math");

test("adds two numbers", () => {
  expect(add(1, 2)).toBe(3);
});

test("doubles a number", () => {
  expect(double(2)).toBe(4);
});
