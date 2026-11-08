"""A sample of Python for the contract tests in `tests/language/`."""

import os
import numpy as np
from . import sibling
from collections import *


class Store:
    """A store of values."""

    limit = 10

    def __init__(self):
        self.values = []

    def put(self, other: Store):
        self.log()
        helper()
        Store.create()
        other.put()
        os.getcwd()

    def log(self):
        pass


def helper():
    # Make an array.
    return np.zeros(1)
