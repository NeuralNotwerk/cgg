function outer()
  local function helper()
  end
  helper()
end
function other()
  helper()
end
