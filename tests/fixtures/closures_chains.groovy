def names = people
    .findAll { it.age >= 18 }
    .collect { p ->
        p.name.toUpperCase()
    }
    .sort()

def grouped = words.groupBy { w -> w[0] }.collectEntries { k, v ->
    [(k): v.size()]
}

[1, 2, 3].each { n ->
    [n, n * 2].each { m ->
        println "${n}:${m}"
    }
}
