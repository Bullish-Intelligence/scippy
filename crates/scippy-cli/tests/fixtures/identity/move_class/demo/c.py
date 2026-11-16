class Widget:
    def run(self, count):
        items = []
        for index in range(count):
            items.append(index * count)
        return items

    def reset(self, value):
        self.value = value
        self.history = [value, value + 1]
        return self.history
