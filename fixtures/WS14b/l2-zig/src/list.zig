pub fn Managed(comptime T: type) type {
    return struct {
        pub fn init(x: u32) @This() {
            _ = x;
            _ = T;
            return .{};
        }
    };
}
