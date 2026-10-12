local Repo = {}
function Repo:zlist(d) return d end
function Repo.szlist(d) return d end
function Repo:caller(data) return zlist(data) + szlist(data) end
local function top(data) return zlist(data) end
