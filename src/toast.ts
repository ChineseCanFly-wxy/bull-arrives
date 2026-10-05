import { createApp } from 'vue';
import { createPinia } from 'pinia';
import ToastWindow from './components/notifications/ToastWindow.vue';
import './assets/styles/variables.css';
createApp(ToastWindow).use(createPinia()).mount('#app');
