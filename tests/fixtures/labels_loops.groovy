def find(List<List<Integer>> grid, int target) {
    outer:
    for (row in grid) {
        for (cell in row) {
            if (cell == target) {
                break outer
            }
            continue
        }
    }
}
