import typing
from typing import Any, Dict, List, Optional


def total(rows: List[int], index: Dict[str, Any], label: Optional[str] = None) -> typing.List[str]:
    return [str(row) for row in rows]
