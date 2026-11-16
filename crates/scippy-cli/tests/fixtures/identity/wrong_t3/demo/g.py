class Gadget:
    def run(self, count):
        names = [str(n) for n in range(count)]
        names.sort(reverse=True)
        return ",".join(names)

    def reset(self, value):
        self.value = None
        self.log = ["reset", value]
        return len(self.log)
