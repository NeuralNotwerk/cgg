const Type = @import("Type.zig");

pub fn field(x: u32) u32 {
    return Type.fromInterned(x);
}
