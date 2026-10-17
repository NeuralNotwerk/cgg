module A where
outer :: Int -> Int
outer x = helper x
  where
    helper y = y + 1

other :: Int -> Int
other z = helper z
