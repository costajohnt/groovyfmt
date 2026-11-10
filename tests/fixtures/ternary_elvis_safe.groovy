def label = user?.profile?.displayName ?:
    user?.login ?:
    'anonymous'
def size = list != null
    ? list.size()
    : 0
