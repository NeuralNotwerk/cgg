program p
  implicit none
  real :: x = 1.0, y(3) = 0.0
  procedure(iface), pointer :: pp => null()
  procedure(iface) :: extp
  dimension :: d(4)
  common /blk/ cm, cn(2)
  type :: t
    real :: comp
  end type
  interface
    function iface(a) result(q)
      real :: a, q
    end function
  end interface
  forall (j = 1:3) y(j) = 0.0
  x = pp(1.0) + extp(2.0) + d(1) + cn(1)
end program
