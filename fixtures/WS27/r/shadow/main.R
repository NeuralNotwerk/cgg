# Functions whose names the parameters and locals below reuse.
cb <- function(x) x
handler <- function(x) x
helper <- function(x) x
step <- function(x) x
made <- function(x) x
later <- function(x) x
num <- function(x) x
now <- function() 0

# Parameter, and a `\(x)` lambda parameter.
with_param <- function(cb) cb(1)
with_lambda <- function(xs) lapply(xs, \(handler) handler(2))

# Locals: a function, an unknown value, `->`, assign() with a literal name.
with_local <- function() {
  helper <- function(x) x * 2
  helper(3)
}
with_unknown <- function(make) {
  made <- make()
  made(4)
}
with_arrow <- function() {
  (function(x) x) -> step
  step(5)
}
with_for <- function(fs) {
  for (later in fs) later(6)
}

# Not locals: `<<-`, a literal value, a top-level assignment.
with_global <- function() {
  cb <<- function(x) x
  cb(7)
}
with_literal <- function() {
  num <- 1
  num(8)
}

# Positive controls: the initializer sees the function, and a call before
# the assignment reaches it.
controls <- function() {
  now()
  handler(9)
  handler <- now()
  handler(10)
}

# A parameter holding a logical, or its own function's result, is passed
# by: R resolves a call to a function binding only.
with_flag <- function(num = TRUE, now = now()) {
  if (num) num(11)
  now()
}
