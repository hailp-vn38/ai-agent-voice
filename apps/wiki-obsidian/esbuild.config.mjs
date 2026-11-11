import esbuild from "esbuild";
import { readFile } from "node:fs/promises";
import { existsSync, readFileSync } from "node:fs";
import { dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { createRequire } from "node:module";
const { parse, compileScript } = createRequire(import.meta.url)("@vue/compiler-sfc");

const production = process.argv[2] === "production";

// Compile the shared Vue panel into the plugin; Obsidian only loads main.js and styles.css.
const vuePlugin = {
  name: "vue-sfc",
  setup(build) {
    build.onLoad({ filter: /\.vue$/ }, async ({ path }) => {
      const { descriptor, errors } = parse(await readFile(path, "utf8"), { filename: path });
      if (errors.length) throw errors[0];
      const compiled = compileScript(descriptor, {
        id: path, inlineTemplate: true,
        fs: { fileExists: existsSync, readFile: file => readFileSync(file, "utf8") },
      });
      return { contents: compiled.content, loader: "ts", resolveDir: dirname(path) };
    });
  },
};

const context = await esbuild.context({
  entryPoints: ["src/main.ts"],
  bundle: true,
  plugins: [vuePlugin],
  alias: { vue: fileURLToPath(new URL("./node_modules/vue/dist/vue.runtime.esm-bundler.js", import.meta.url)) },
  define: { __VUE_OPTIONS_API__: "true", __VUE_PROD_DEVTOOLS__: "false", __VUE_PROD_HYDRATION_MISMATCH_DETAILS__: "false", "process.env.NODE_ENV": JSON.stringify(production ? "production" : "development") },
  external: ["obsidian", "electron", "@codemirror/*", "@lezer/*"],
  format: "cjs",
  target: "es2022",
  sourcemap: production ? false : "inline",
  minify: production,
  treeShaking: true,
  outfile: "main.js",
  logLevel: "info",
});

const styles = await esbuild.context({
  entryPoints: ["src/styles.css"], bundle: true, minify: production, outfile: "styles.css",
});

if (production) {
  await context.rebuild();
  await styles.rebuild();
  await context.dispose();
  await styles.dispose();
} else {
  await context.watch();
  await styles.watch();
}
