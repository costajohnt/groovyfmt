def config = [
    name   : 'svc',
    ports  : [8080, 8443],
    nested : [
        enabled: true,
        tags   : ['a', 'b'],
    ],
]
def empty = [:]
def matrix = [
    [1, 0],
    [0, 1]
]
