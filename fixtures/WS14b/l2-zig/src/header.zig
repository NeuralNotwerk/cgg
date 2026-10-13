pub const Header = struct {
    fn Functions(comptime H: type) type {
        return struct {
            pub fn frame_const(h: *const H) u32 {
                _ = h;
                return 0;
            }
        };
    }

    pub const Ping = struct {
        id: u32,
        pub const frame_const = Functions(@This()).frame_const;
    };
};
