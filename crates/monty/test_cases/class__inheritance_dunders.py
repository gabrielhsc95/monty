# Test inheritance of special "dunder" methods

class ContainerBase:
    def __init__(self, data):
        self.data = data

    def __str__(self):
        return "Container(" + str(self.data) + ")"

    def __eq__(self, other):
        if type(self) is type(other):
            return self.data == other.data
        return False

    def __contains__(self, item):
        return item in self.data

    def __iter__(self):
        return iter(self.data)

class MyContainer(ContainerBase):
    # Inherits all dunders from ContainerBase!
    def is_empty(self):
        return len(self.data) == 0

# Test __init__ inheritance
c1 = MyContainer([1, 2, 3])
c2 = MyContainer([1, 2, 3])
c3 = MyContainer([4, 5])

# Test __str__ inheritance
assert str(c1) == "Container([1, 2, 3])"

# Test __eq__ inheritance
assert c1 == c2
assert not (c1 == c3)

# Test __contains__ inheritance
assert 2 in c1
assert 9 not in c1

# Test __iter__ inheritance
items = []
for x in c3:
    items.append(x)
assert items == [4, 5]

# Test own method
assert not c1.is_empty()

print("success")
