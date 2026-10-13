const Pool = @This();
const Other = @import("other.zig");

items: u32,

pub fn init() Pool {
    return .{ .items = 0 };
}

pub fn deinit(self: *Pool) void {
    _ = self;
}

pub const Inner = struct {
    pub fn deinit(self: *Inner) void {
        _ = self;
    }
    fn make() Inner {
        return .{};
    }
    fn use() void {
        var i = make();
        i.deinit();
    }
};

fn useIt() void {
    var p = Pool.init();
    p.deinit();
    const o = Other.get();
    o.deinit();
    var q: Inner = undefined;
    q.missing();
}
