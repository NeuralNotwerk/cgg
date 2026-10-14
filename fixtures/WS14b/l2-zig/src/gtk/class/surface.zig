const Common = @import("../class.zig").Common;
pub const Surface = struct {
    const Self = @This();
    const Private = struct {};
    const C = Common(Self, Private);
    pub const as = C.as;
};
