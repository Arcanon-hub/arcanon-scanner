import { Controller, Get, Post } from '@nestjs/common';

@Controller('/api/v1')
export class AppController {
  @Get('/users')
  getUsers() {
    return [];
  }

  @Post('/users')
  createUser() {
    return {};
  }
}
