import { createApp } from "vue";
import { createPinia } from "pinia";
import {
  NButton,
  NCheckbox,
  NConfigProvider,
  NDrawer,
  NDrawerContent,
  NDialogProvider,
  NDivider,
  NDropdown,
  NInput,
  NInputNumber,
  NMessageProvider,
  NModal,
  NPopconfirm,
  NRadioGroup,
  NRadioButton,
  NSelect,
  NSwitch,
  NTab,
  NTabs,
  NTag,
  NTooltip,
} from "naive-ui";
import App from "./App.vue";
import { installErrorOverlay } from "./error-overlay";
import "./styles/reset.css";
import "./styles/tokens.css";

installErrorOverlay();

const app = createApp(App);

app.use(createPinia());
const components = {
  NButton,
  NCheckbox,
  NConfigProvider,
  NDrawer,
  NDrawerContent,
  NDialogProvider,
  NDivider,
  NDropdown,
  NInput,
  NInputNumber,
  NMessageProvider,
  NModal,
  NPopconfirm,
  NRadioGroup,
  NRadioButton,
  NSelect,
  NSwitch,
  NTab,
  NTabs,
  NTag,
  NTooltip,
};
for (const [name, comp] of Object.entries(components)) {
  app.component(name, comp);
}

app.mount("#app");