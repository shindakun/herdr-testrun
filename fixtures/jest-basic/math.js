function add(a, b) {
  return a + b;
}

// Wrong on purpose.
function double(n) {
  return n + n + 1;
}

module.exports = { add, double };
