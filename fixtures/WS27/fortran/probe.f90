module m
  real :: modarr(10)
contains
  function f(x, cb) result(r)
    real, intent(in) :: x
    procedure(iface) :: cb
    real :: r, a(10)
    integer, dimension(:), allocatable :: b
    real c(5)
    external :: ext
    sf(y) = y * 2
    r = a(1) + b(2) + c(3) + cb(x) + g(x) + sf(x)
    associate (z => a(1))
      r = z
    end associate
    block
      integer :: k
      k = 1
    end block
    do concurrent (i = 1:3)
      r = r + i
    end do
    call ext(r)
  contains
    subroutine inner()
      r = a(2)
    end subroutine
  end function
end module
