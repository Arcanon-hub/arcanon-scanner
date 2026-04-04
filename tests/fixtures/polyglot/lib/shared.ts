// Shared utility library — no Dockerfile, should be UNSCOPED
import axios from 'axios';

export async function callExternalApi(url: string) {
  const response = await axios.get(url);
  return response.data;
}
