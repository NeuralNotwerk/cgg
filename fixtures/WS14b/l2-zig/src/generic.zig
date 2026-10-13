const std = @import("std");
const list = @import("list.zig");

const Attr = struct {
    fn init() Attr {
        return .{};
    }
};

pub fn build() void {
    var a = list.Managed(Attr).init(1);
    _ = a;
    var b = std.array_list.Managed(Attr).init(1);
    _ = b;
}
