class Parent:
    x = 1

class Child(Parent):
    pass

c = Child()

# We test that accessing a missing attribute correctly traverses
# the MRO and ultimately raises an AttributeError.
try:
    val = c.y
    raised = False
except AttributeError as e:
    raised = True
    # Verify the error message matches exactly
    assert str(e) == "'Child' object has no attribute 'y'"

assert raised, "Expected AttributeError to be raised, but nothing was."

print("success")
