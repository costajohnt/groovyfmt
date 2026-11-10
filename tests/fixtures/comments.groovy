/*
 * Licensed to nobody in particular.
 */
package util

// helpers for dates
class Dates {
// commented out at column 0
//    static old() {}
    /**
     * Days between two dates.
     */
    static long between(a, b) {
        // inclusive
        return b - a /* exclusive */ + 1
    }
}
