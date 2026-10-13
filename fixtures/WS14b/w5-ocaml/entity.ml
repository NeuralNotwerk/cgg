module IsAdmin = struct
  include Base.Boolean
  let schema = schema
  let create v = create v
end

module Other = struct
  let schema () = 2
end

let rec loop n = if n = 0 then 0 else loop (n - 1)
let helper x = x + 1
let helper x = helper (x * 2)
