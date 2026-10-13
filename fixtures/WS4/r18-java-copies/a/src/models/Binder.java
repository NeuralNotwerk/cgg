package models;

public class Binder {
    // Same-package class, copies of models.User elsewhere: this copy's dir wins.
    public User make() { return new User(); }
}
