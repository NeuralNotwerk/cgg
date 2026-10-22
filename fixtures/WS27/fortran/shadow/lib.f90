! Functions whose names the arrays, dummies and locals in main.f90 reuse.
module lib
contains
  real function weights(i)
    integer :: i
    weights = i
  end function
  real function cb(x)
    real :: x
    cb = x
  end function
  real function tbl(i)
    integer :: i
    tbl = i
  end function
  real function scale(x)
    real :: x
    scale = x
  end function
  real function helper(x)
    real :: x
    helper = x
  end function
end module
