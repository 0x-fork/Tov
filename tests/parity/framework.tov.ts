// The Tov port of framework.ts: the same app, with `Js` for `any`. Diff them to see every edit.
import { createApp } from "parity-framework"

class Refused extends Error {
  constructor(message: string) {
    super(message)
    this.name = "Refused"
  }
}

function user(ctx: Js): Js {
  const p = ctx.params
  return `hello ${p.name}`
}

function readName(ctx: Js): Js {
  return ctx.params.name
}

function helper(ctx: Js): Js {
  const name = readName(ctx)
  return `helper ${name}`
}

function guarded(ctx: Js): Js {
  try {
    return `guarded ${ctx.params.name}`
  } catch (e) {
    return `guarded caught ${e.name}: ${e.message}`
  }
}

async function main() {
  const app = createApp()
  app.route("/users/:name", user)
  app.route("/helper/:name", helper)
  app.route("/guarded/:name", guarded)
  app.route("/async/:name", async (ctx: Js): Promise<Js> => {
    await Promise.resolve(0)
    return `async ${ctx.params.name}`
  })
  app.route("/fail", (ctx: Js) => ctx.fail("bang"))
  app.route("/refused", (ctx: Js): Js => {
    throw new Refused("not today")
  })
  app.route("/query", (ctx: Js) => `query ${ctx.query.q}`)
  app.listen(Number(process.env.PORT ?? 3000))
}
