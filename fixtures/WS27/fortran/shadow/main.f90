module app
  use lib
contains
  ! An array, a dummy procedure, an array declared in another case.
  real function use_array(n)
    integer :: n
    real :: weights(10)
    weights = 1.0
    use_array = weights(n)
  end function
  real function use_dummy(cb, x)
    procedure(cb_iface) :: cb
    real :: x
    use_dummy = cb(x)
  end function
  real function use_case(i)
    integer :: i
    REAL :: Tbl(3)
    Tbl = 0.0
    use_case = tbl(i)
  end function
  ! A statement function.
  real function use_stmt(x)
    real :: x, scale
    scale(x) = x * 2.0
    use_stmt = scale(x)
  end function
  ! Positive control: a real call to a module function.
  real function use_call(x)
    real :: x
    use_call = helper(x)
  end function
end module
