// Class inheritance inside Monty: single and multiple inheritance with C3 MRO.
//
// Sandbox code can define class hierarchies. Instances returned to the host
// are read-only `MontyClassProxy` snapshots.

import assert from 'node:assert/strict'
import { Monty, MontyClassProxy } from '@pydantic/monty'

const code = `\
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
`

await using pool = await Monty.create()
await using session = await pool.checkout()
const result = await session.feedRun(code)

assert.ok(result instanceof MontyClassProxy)
assert.equal(result.name, 'Sub')
assert.deepEqual({ ...result.attributes }, { name: 'World' })
console.log(`host received: ${result.name} ${JSON.stringify(result.attributes)}`)
