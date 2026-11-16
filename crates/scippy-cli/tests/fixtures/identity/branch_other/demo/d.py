def feature_only(path, mode):
    parts = path.split("/")
    head = parts[0]
    tail = "/".join(parts[1:])
    if mode == "strict":
        return head, tail
    return tail, head
