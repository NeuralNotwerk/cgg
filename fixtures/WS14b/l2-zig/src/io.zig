pub const IO = struct {
    fd: u32,
    pub fn init(n: u32) !IO {
        return .{ .fd = n };
    }
    pub fn deinit(self: *IO) void {
        _ = self;
    }
};
