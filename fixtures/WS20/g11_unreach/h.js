function target() { return 1; }

function outer() {
  return inner();
  function inner() { return target(); }
}

outer();
