pub fn Common(comptime Self: type, comptime Private: ?type) type {
    return struct {
        pub fn as(self: *Self, comptime T: type) *T {
            _ = Private;
            return @ptrCast(self);
        }
        pub const Class = struct {
            pub fn as(class: *Self.Class, comptime T: type) *T {
                return @ptrCast(class);
            }
        };
    };
}
