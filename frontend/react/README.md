# React frontend

The plan chose a React frontend. It is a Vite app (TypeScript, React
19.3.0) built by Bazel with rules_js into `dist/`, which the
workflow uploads as the `static` artifact: the platform serves it at `/` from
the environment's public bucket, answering a path that names no file with
`index.html` (so the app's own routes load), and the HTTPRoute sends `/api/`
to the app and `/.ory/` to Kratos. There is no frontend
image and no frontend pod.

| Target | What it does |
|---|---|
| `//frontend/react:build` | the production bundle (`dist/`), the `static` artifact's content |
| `//frontend/react:typecheck` | `tsc` over `src/`: Vite strips types without checking them, so this test is what fails on a type error |
| `//frontend/react:proto_es` | the Connect client, generated from every `.proto` under `proto/` into `src/gen/` |
| `//frontend/react:start` | the dev server (`ibazel run`), `/api` proxied to an app on localhost:8080 |

## Calling the API

Every nanoservice API is a proto service under `proto/`; the build generates
its TypeScript descriptors into `src/gen/<package path>/<file>_pb.ts`. Create
one client per service in `src/api.ts`, next to the health client:

```ts
import { OrderService } from "./gen/order/v1/order_pb";
export const orders = createClient(OrderService, transport);
```

Never write `src/gen/` by hand and never commit it; change the `.proto` file
and the next build regenerates it.

## Dependencies

`package.json` pins exact versions and `pnpm-lock.yaml` is the resolved
graph rules_js builds from. To add a package, add it to `package.json`: the
verify build (`.basable/verify.sh`) resolves it into the lockfile and commits
the refreshed lock. By hand: `bazel run -- @pnpm --dir "$PWD/frontend/react"
install --lockfile-only`. Nothing in `BUILD.bazel` changes, since the build
depends on `:node_modules` as a whole. A package with an install script
needs an explicit answer (`allowBuilds`) in a `pnpm-workspace.yaml` next to
the lockfile; rules_js names the package when it is missing.
