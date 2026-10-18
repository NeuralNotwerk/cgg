module shapes
  interface
    function area(x) result(r)
      real :: x, r
    end function area
  end interface
contains
  function area(x) result(r)
    real :: x, r
    r = x
  end function area
  subroutine run()
    real :: y
    y = area(1.0)
  end subroutine run
end module shapes
