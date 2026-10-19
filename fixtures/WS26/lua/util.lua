local M = {}
function M.cmp(a, b) return a < b end
function M.unused() end            -- control: dead
return M
