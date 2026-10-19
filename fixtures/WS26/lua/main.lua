local util = require("util")

local function protected() return 1 end
local function on_event() end
local function stored() end
local function orphan() end         -- control: dead
local function shadowed() end       -- near-miss: only a parameter of this name is passed
local function in_string() end      -- near-miss: only a string

local function use(shadowed) invoke(shadowed) end
local function invoke(f) f() end

local function main(t)
  pcall(protected)
  table.sort(t, util.cmp)
  local h = { handler = on_event }
  local obj = {}
  obj.cb = stored
  print("in_string")
  use(nil)
  return h, obj
end

main({})
