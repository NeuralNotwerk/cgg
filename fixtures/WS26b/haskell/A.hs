module A where
import B (helper)
handler x = x + 1
run xs = map handler xs
shadow handler = map handler [1]
composed = handler . helper
w = q
  where q = handler
w2 = q2 1
  where q2 y = handler y
