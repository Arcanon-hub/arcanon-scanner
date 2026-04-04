import { Controller, Get, Post } from '@nestjs/common';

@Controller('/users')
export class UsersController {
  @Get('/:id')
  getUser(id: string) {
    return { id };
  }

  @Post('/')
  createUser() {
    return {};
  }
}
