"""Class inheritance inside Monty: single and multiple inheritance with C3 MRO.

Sandbox code can define class hierarchies. Instances returned to the host
are read-only `MontyClassProxy` snapshots with attributes resolved through MRO.
"""

from pydantic_monty import Monty, MontyClassProxy

CODE = """\
class Base:
    greeting = 'Hello'

    def __init__(self, name: str):
        self.name = name

    def message(self) -> str:
        return f'{self.greeting}, {self.name}!'

class Sub(Base):
    greeting = 'Welcome'

s = Sub('World')
assert s.message() == 'Welcome, World!'
s
"""

with Monty() as pool:
    with pool.checkout() as session:
        result = session.feed_run(CODE)

assert isinstance(result, MontyClassProxy)
assert result.name == 'Sub'
assert result.attributes == {'name': 'World'}
print(f'host received: {result!r}')
