module Main where

-- Top-level functions whose names the bindings below reuse.
cb x = x
go x = x
step x = x
item x = x
found x = x
alt x = x
done x = x
tmp x = x
now x = x

-- An argument pattern, a lambda, a `do` bind and a `case` alternative.
withArg cb = cb 1
withLambda = \step -> step 2
withDo = do
  item <- pure id
  pure (item 3)
withCase m = case m of
  Just alt -> alt 4
  Nothing -> 0

-- A pattern guard, an expression `let`, a `where` value.
withGuard m
  | Just found <- m = found 5
  | otherwise = 0
withLet = let done = id in done 6
withWhereValue = tmp 7
  where
    tmp = id

-- A `where` function with a top-level twin: calls bind the `where` one.
withWhere x = go x
  where
    go y = y + 1

-- Positive controls: the binding's own scope ends; another clause.
controls = now 8
otherClause = go 9
