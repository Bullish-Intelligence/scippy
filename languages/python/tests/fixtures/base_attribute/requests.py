from typing import Tuple


class Request:
    attributes: Tuple[str, ...] = ("url",)


class JsonRequest(Request):
    attributes: Tuple[str, ...] = Request.attributes + ("dumps_kwargs",)

    def own(self):
        return self.attributes

    def other(self, request: Request):
        return request.attributes
