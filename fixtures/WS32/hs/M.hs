module Main where
import qualified Data.Map as M

main :: IO ()
main = do
  r <- run 1
  print r

run :: Int -> IO (M.Map Int Int)
run n = do
  remaining <- pure M.empty
  let
    f a b = a + b
  return $ M.insertWith f n 1 remaining

other :: Int -> Int
other x = f x
  where f y = y
