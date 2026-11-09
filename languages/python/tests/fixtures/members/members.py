class Options:
    def __init__(self):
        self.cache_dir = ""
        self.depth = 1


class Server:
    def __init__(self):
        self.options = Options()

    def read(self):
        # `cache_dir` is a local variable. The member `cache_dir` is the field.
        cache_dir = self.options.cache_dir
        self.options.cache_dir = cache_dir
        return cache_dir

    def local_class(self):
        # A class that a function holds keeps its attributes.
        class Local:
            depth = 2

            def __init__(self):
                self.cache_dir = "x"

        item = Local()
        return item.depth, item.cache_dir
