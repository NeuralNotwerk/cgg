local function pcallTarget() return 1 end
local function cmpFn(a, b) return a < b end
local function assigned() return 2 end
local function deadControl() return 0 end
local function calledControl() return 3 end
local M = {}
function M.tableFn() return 4 end
local ok = pcall(pcallTarget)
local t = {3, 1}
table.sort(t, cmpFn)
local h = assigned
local handlers = { a = M.tableFn }
calledControl()
return M
