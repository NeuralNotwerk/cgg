function outer()
  local function go()
  end
  go()
end
function other()
  local f = function(go) go() end
  f(outer)
end
