local M = {}
function M.f()
  return M.g()
end
function M.g()
  return 1
end
function M:h()
  return self:f()
end
return M
