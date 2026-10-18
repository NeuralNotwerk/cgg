package nearmiss

// A same-named annotation that is NOT Spring's: no spring import here,
// and the repo-level detection is per-rule, so this file's own
// annotation class decides nothing by itself.
annotation class Scheduled

class Local {
    @Scheduled
    fun localTick() {}
}
