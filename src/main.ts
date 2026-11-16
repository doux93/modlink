import { createApp } from "vue";
import { createPinia } from "pinia";
import App from "./App.vue";
import { installErrorOverlay } from "./error-overlay";
import "./styles/reset.css";
import "./styles/tokens.css";

installErrorOverlay();

const app = createApp(App);

app.use(createPinia());
app.mount("#app");
