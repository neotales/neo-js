import starlight from "@astrojs/starlight";
import { defineConfig } from "astro/config";

export default defineConfig({
	integrations: [
		starlight({
			title: "neo-js",
			sidebar: [
				{ label: "Run a script", slug: "guides/run" },
				{ label: "Runtime", items: [{ autogenerate: { directory: "runtime" } }] },
				{ label: "Module guides", items: [{ autogenerate: { directory: "guides/modules" } }] },
				{ label: "API reference", slug: "api" },
				{ label: "Globals", slug: "api/globals" },
				{
					label: "Module declarations",
					collapsed: true,
					items: [{ autogenerate: { directory: "api/modules" } }],
				},
			],
		}),
	],
});
