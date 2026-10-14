local M = {}
M.sub = {}
function M.sub.deep()
  return 1
end
M.assigned = function(x)
  return M.sub.deep()
end
local t = {
  field = function() return M.assigned(1) end,
}
function M:meth()
  return self:other()
end
function M:other() return 1 end
local function loc() return M.assigned(2) end
return M
